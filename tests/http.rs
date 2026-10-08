use rust_jev::{rust_decision::*, *};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct Mock {
    endpoint: String,
    seen: mpsc::Receiver<(String, Value)>,
    worker: thread::JoinHandle<usize>,
}
fn mock(status: u16, body: String, headers: &str, delay: Duration, chunked: bool) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let headers = headers.to_owned();
    let (tx, seen) = mpsc::channel();
    let worker = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let start = Instant::now();
        let mut calls = 0;
        while start.elapsed() < Duration::from_secs(3) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    calls += 1;
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let (header, payload) = read_request(&mut stream);
                    tx.send((header, serde_json::from_slice(&payload).unwrap()))
                        .unwrap();
                    thread::sleep(delay);
                    let encoding = if chunked {
                        "Transfer-Encoding: chunked\r\n".into()
                    } else {
                        format!("Content-Length: {}\r\n", body.len())
                    };
                    let wire_body = if chunked {
                        format!("{:x}\r\n{}\r\n0\r\n\r\n", body.len(), body)
                    } else {
                        body.clone()
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\n{encoding}{headers}Connection: close\r\n\r\n{wire_body}"
                    );
                    // Watch for a hidden retry after the response/transport failure.
                    let end = Instant::now() + Duration::from_millis(100);
                    while Instant::now() < end {
                        match listener.accept() {
                            Ok((_stream, _)) => calls += 1,
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(5))
                            }
                            Err(e) => panic!("{e}"),
                        }
                    }
                    return calls;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(e) => panic!("{e}"),
            }
        }
        calls
    });
    Mock {
        endpoint,
        seen,
        worker,
    }
}
fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut raw = vec![];
    let mut buf = [0; 4096];
    loop {
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0);
        raw.extend_from_slice(&buf[..n]);
        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let header = String::from_utf8(raw[..end].to_vec()).unwrap();
            let length: usize = header
                .lines()
                .find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse().unwrap())
                })
                .unwrap();
            if raw.len() >= end + 4 + length {
                return (header, raw[end + 4..end + 4 + length].to_vec());
            }
        }
    }
}
fn config(endpoint: &str) -> Config {
    let mut c = Config::new("test-secret", "jev-requested")
        .unwrap()
        .allow_loopback_http();
    c.endpoint = endpoint.into();
    c.provider_label = "mock".into();
    c
}
fn request() -> Request<u8> {
    Request {
        question_id: "category".into(),
        context: "A message".into(),
        instructions: "Choose category".into(),
        options: vec![
            Choice {
                id: "z".into(),
                description: "Policy".into(),
                value: 1,
            },
            Choice {
                id: "a".into(),
                description: "Mechanics".into(),
                value: 2,
            },
        ],
    }
}
fn response() -> Value {
    json!({"model":"jev-returned","answers":{"category":{"type":"choice","choice":"z","probabilities":{"a":0.2,"z":0.8},"confidence":0.9}},"usage":{"input_tokens":10,"output_tokens":0}})
}
fn one(body: String, policy: Policy) -> JevReport<u8> {
    let server = mock(200, body, "", Duration::ZERO, false);
    let mut client = JevClient::new(config(&server.endpoint)).unwrap();
    let report = client.decide(&request(), &policy, Observation::default);
    assert_eq!(server.worker.join().unwrap(), 1);
    report
}
#[test]
fn maps_payload_answer_and_provenance_to_local_value() {
    let server = mock(
        200,
        response().to_string(),
        "X-TypeSafe-Request-Id: vendor-42\r\nX-Request-Id: gateway-7\r\n",
        Duration::ZERO,
        false,
    );
    let mut client = JevClient::new(config(&server.endpoint)).unwrap();
    let report = client.decide(
        &request(),
        &Policy {
            min_probability: Some(0.7),
            ..Default::default()
        },
        Observation::default,
    );
    assert_eq!(report.core.decision, Decision::Accepted(1));
    assert_eq!(report.core.rules.len(), 9);
    let metadata = report.metadata.unwrap();
    assert_eq!(metadata.requested_model, "jev-requested");
    assert_eq!(metadata.returned_model, "jev-returned");
    assert_eq!(metadata.typesafe_request_id.as_deref(), Some("vendor-42"));
    assert_eq!(metadata.gateway_request_id.as_deref(), Some("gateway-7"));
    assert_eq!(
        metadata.usage,
        Some(Usage {
            input_tokens: Some(10),
            output_tokens: Some(0)
        })
    );
    let (headers, body) = server.seen.recv().unwrap();
    assert!(headers.starts_with("POST /v1/systemone HTTP/1.1"));
    assert!(
        headers
            .to_lowercase()
            .contains("authorization: bearer test-secret")
    );
    assert_eq!(
        body,
        json!({"model":"jev-requested","state":"A message","questions":{"category":{"type":"choice","instructions":"Choose category","criteria":{"z":"Policy","a":"Mechanics"}}}})
    );
    let order: Vec<_> = body["questions"]["category"]["criteria"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(order, ["z", "a"]);
    assert_eq!(server.worker.join().unwrap(), 1);
}
#[test]
fn supports_explicit_object_state_without_text_autodetection() {
    for encoding in [StateEncoding::Text, StateEncoding::Json] {
        let server = mock(200, response().to_string(), "", Duration::ZERO, false);
        let mut c = config(&server.endpoint);
        c.state_encoding = encoding;
        let mut client = JevClient::new(c).unwrap();
        let mut r = request();
        r.context = r#"{"candidate":"code","reference":"spec"}"#.into();
        assert_eq!(
            client
                .decide(&r, &Policy::default(), Observation::default)
                .core
                .decision,
            Decision::Accepted(1)
        );
        let (_, body) = server.seen.recv().unwrap();
        assert_eq!(
            body["state"],
            match encoding {
                StateEncoding::Text => json!(r.context),
                StateEncoding::Json => json!({"candidate":"code","reference":"spec"}),
            }
        );
        assert_eq!(server.worker.join().unwrap(), 1);
    }
}
#[test]
fn malformed_identity_types_and_duplicate_keys_fail_closed() {
    let mut cases=vec!["not json".into(),"null".into(),r#"{"model":"jev","answers":{"category":{"type":"choice","choice":"z","confidence":0.9,"probabilities":{"z":0.8,"z":0.7,"a":0.2}}}}"#.into()];
    for change in 0..7 {
        let mut body = response();
        match change {
            0 => {
                body["answers"] = json!({"wrong":body["answers"]["category"].clone()});
            }
            1 => {
                body["answers"]["extra"] = body["answers"]["category"].clone();
            }
            2 => {
                body["answers"]["category"]["type"] = "noul".into();
            }
            3 => {
                body["answers"]["category"]
                    .as_object_mut()
                    .unwrap()
                    .remove("confidence");
            }
            4 => {
                body["answers"]["category"]["probabilities"] = json!({"z":0.8,"wrong":0.2});
            }
            5 => {
                body["answers"]["category"]["confidence"] = "0.9".into();
            }
            _ => {
                body["usage"]["input_tokens"] = (-1).into();
            }
        }
        cases.push(body.to_string());
    }
    for body in cases {
        let r = one(body, Policy::default());
        assert_eq!(
            r.core.decision,
            Decision::Failed(Failure::MalformedResponse)
        );
        assert_eq!(r.adapter_error, Some(Error::MalformedResponse));
        assert_eq!(r.core.rules.len(), 3);
    }
}
#[test]
fn numeric_validation_and_abstention_remain_core_rules() {
    for value in [-0.1, 1.1] {
        let mut body = response();
        body["answers"]["category"]["confidence"] = value.into();
        let r = one(body.to_string(), Policy::default());
        assert_eq!(r.core.decision, Decision::Failed(Failure::OutputValidation));
        assert_eq!(r.adapter_error, None);
        assert_eq!(
            r.core.rules.last().unwrap().rule,
            "AdvertisedConfidenceValid"
        );
    }
    let r = one(
        response().to_string(),
        Policy {
            min_confidence: Some(0.95),
            ..Default::default()
        },
    );
    assert_eq!(r.core.decision, Decision::Abstained(Reason::PolicyRejected));
    assert!(r.metadata.is_some());
    let mut body = response();
    body["answers"]["category"]["choice"] = "unknown".into();
    assert_eq!(
        one(body.to_string(), Policy::default()).core.decision,
        Decision::Failed(Failure::OutputValidation)
    );
}
#[test]
fn failures_keep_safe_categories_without_fallback_or_retry() {
    for (status, failure) in [
        (401, Failure::Authentication),
        (403, Failure::Authentication),
        (422, Failure::UnsupportedCapability),
        (429, Failure::Transport),
        (500, Failure::Transport),
        (504, Failure::TimedOut),
        (307, Failure::Transport),
    ] {
        let server = mock(
            status,
            "SENSITIVE BODY".into(),
            "Location: https://never-follow.invalid/\r\n",
            Duration::ZERO,
            false,
        );
        let mut client = JevClient::new(config(&server.endpoint)).unwrap();
        let p = Policy {
            fallback: Some(Fallback {
                option_id: "a".into(),
                triggers: vec![Reason::PolicyRejected, Reason::ProviderRefusal],
            }),
            ..Default::default()
        };
        let report = client.decide(&request(), &p, Observation::default);
        assert_eq!(report.core.decision, Decision::Failed(failure));
        assert_eq!(report.adapter_error, Some(Error::Http(status)));
        assert!(!format!("{report:?}").contains("SENSITIVE BODY"));
        assert!(!format!("{client:?}").contains("test-secret"));
        assert_eq!(server.worker.join().unwrap(), 1);
    }
}
#[test]
fn bounds_response_with_and_without_content_length() {
    for chunked in [false, true] {
        let server = mock(200, "x".repeat(100), "", Duration::ZERO, chunked);
        let mut c = config(&server.endpoint);
        c.max_response_bytes = 20;
        let report =
            JevClient::new(c)
                .unwrap()
                .decide(&request(), &Policy::default(), Observation::default);
        assert_eq!(report.adapter_error, Some(Error::ResponseTooLarge));
        assert_eq!(
            report.core.decision,
            Decision::Failed(Failure::MalformedResponse)
        );
        assert_eq!(server.worker.join().unwrap(), 1);
    }
}
#[test]
fn times_out_waiting_for_headers() {
    let server = mock(
        200,
        response().to_string(),
        "",
        Duration::from_millis(800),
        false,
    );
    let mut c = config(&server.endpoint);
    c.timeout = Duration::from_millis(200);
    c.connect_timeout = c.timeout;
    let r = JevClient::new(c)
        .unwrap()
        .decide(&request(), &Policy::default(), Observation::default);
    assert_eq!(r.core.decision, Decision::Failed(Failure::TimedOut));
    assert_eq!(r.adapter_error, Some(Error::TimedOut));
    assert_eq!(server.worker.join().unwrap(), 1);
}
#[test]
fn optional_usage_is_unknown_and_no_stale_metadata_on_skipped_invocation() {
    let mut body = response();
    body.as_object_mut().unwrap().remove("usage");
    let server = mock(200, body.to_string(), "", Duration::ZERO, false);
    let mut client = JevClient::new(config(&server.endpoint)).unwrap();
    let r = client.decide(&request(), &Policy::default(), Observation::default);
    assert_eq!(r.metadata.unwrap().usage, None);
    let r = client.decide(&request(), &Policy::default(), || Observation {
        cancelled: true,
        expired: false,
    });
    assert_eq!(r.core.decision, Decision::Cancelled);
    assert_eq!(r.core.invocations, 0);
    assert!(r.metadata.is_none());
    assert!(r.adapter_error.is_none());
    assert!(r.prediction.is_none());
    assert_eq!(server.worker.join().unwrap(), 1);
}
#[test]
fn invalid_configuration_and_keys_are_redacted() {
    for key in ["", "contains space", "line\nkey", "é"] {
        assert!(matches!(Config::new(key, "jev"), Err(Error::Configuration)));
    }
    for url in [
        "http://127.0.0.1:1/",
        "http://example.com/",
        "https://user:secret@example.com/",
        "https://example.com/?key=secret",
        "https://example.com/#secret",
    ] {
        let mut c = Config::new("test-secret", "jev").unwrap();
        c.endpoint = url.into();
        assert!(matches!(JevClient::new(c), Err(Error::Configuration)));
    }
    let mut c = config("http://example.com/");
    assert!(matches!(JevClient::new(c), Err(Error::Configuration)));
    c = config("http://127.0.0.1:1/");
    c.timeout = Duration::ZERO;
    assert!(matches!(JevClient::new(c), Err(Error::Configuration)));
    let c = Config::new("test-secret", "jev").unwrap();
    assert!(!format!("{c:?}").contains("test-secret"));
}
#[test]
fn encoded_request_quota_is_checked_before_connecting() {
    let mut c = config("http://127.0.0.1:1/");
    c.max_request_bytes = 80;
    let r = JevClient::new(c)
        .unwrap()
        .decide(&request(), &Policy::default(), Observation::default);
    assert_eq!(r.adapter_error, Some(Error::RequestTooLarge));
    assert_eq!(
        r.core.decision,
        Decision::Failed(Failure::UnsupportedCapability)
    );
}

#[test]
fn times_out_while_reading_response_body() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        read_request(&mut stream);
        let body = response().to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.flush().unwrap();
        thread::sleep(Duration::from_millis(800));
        let _ = stream.write_all(body.as_bytes());
    });
    let mut c = config(&endpoint);
    c.timeout = Duration::from_millis(200);
    c.connect_timeout = c.timeout;
    let r = JevClient::new(c)
        .unwrap()
        .decide(&request(), &Policy::default(), Observation::default);
    assert_eq!(r.core.decision, Decision::Failed(Failure::TimedOut));
    assert_eq!(r.adapter_error, Some(Error::TimedOut));
    worker.join().unwrap();
}
#[test]
fn total_timeout_includes_headers_and_continuously_progressing_body() {
    for chunked in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            read_request(&mut stream);
            let body = response().to_string();
            // Both the header delay and each read delay fit within the budget.
            // Only a timer covering the entire request rejects this response.
            thread::sleep(Duration::from_millis(200));
            let encoding = if chunked {
                "Transfer-Encoding: chunked\r\n".into()
            } else {
                format!("Content-Length: {}\r\n", body.len())
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\n{encoding}Connection: close\r\n\r\n"
            )
            .unwrap();
            stream.flush().unwrap();
            for part in body.as_bytes().chunks(8) {
                thread::sleep(Duration::from_millis(80));
                let result = if chunked {
                    write!(stream, "{:x}\r\n", part.len())
                        .and_then(|_| stream.write_all(part))
                        .and_then(|_| stream.write_all(b"\r\n"))
                } else {
                    stream.write_all(part)
                };
                if result.and_then(|_| stream.flush()).is_err() {
                    return;
                }
            }
            if chunked {
                let _ = stream.write_all(b"0\r\n\r\n");
            }
        });
        let mut c = config(&endpoint);
        c.timeout = Duration::from_millis(600);
        c.connect_timeout = c.timeout;
        let report =
            JevClient::new(c)
                .unwrap()
                .decide(&request(), &Policy::default(), Observation::default);
        worker.join().unwrap();
        assert_eq!(report.adapter_error, Some(Error::TimedOut));
        assert_eq!(report.core.decision, Decision::Failed(Failure::TimedOut));
        assert_eq!(report.core.invocations, 1);
        assert!(report.metadata.is_none());
    }
}
#[test]
fn malformed_explicit_state_and_duplicate_answers_are_rejected() {
    for state in ["null", "true", "123", r#"{"code":1,"code":2}"#] {
        let mut c = config("http://127.0.0.1:1/");
        c.state_encoding = StateEncoding::Json;
        let mut request = request();
        request.context = state.into();
        let report =
            JevClient::new(c)
                .unwrap()
                .decide(&request, &Policy::default(), Observation::default);
        assert_eq!(report.adapter_error, Some(Error::InvalidRequest));
    }
    let answer =
        r#"{"type":"choice","choice":"z","confidence":0.9,"probabilities":{"z":0.8,"a":0.2}}"#;
    let body =
        format!(r#"{{"model":"jev","answers":{{"category":{answer},"category":{answer}}}}}"#);
    assert_eq!(
        one(body, Policy::default()).adapter_error,
        Some(Error::MalformedResponse)
    );
}
#[test]
fn preserves_unknown_usage_counts_and_reconciles_probabilities_in_request_order() {
    let mut body = response();
    body["usage"] = json!({"input_tokens":null});
    let server = mock(200, body.to_string(), "", Duration::ZERO, false);
    let mut client = JevClient::new(config(&server.endpoint)).unwrap();
    let core_request = request();
    let request = BackendRequest {
        question_id: core_request.question_id,
        context: core_request.context,
        instructions: core_request.instructions,
        options: core_request
            .options
            .into_iter()
            .map(|o| (o.id, o.description))
            .collect(),
    };
    match client.invoke(&request) {
        BackendEvent::Prediction(p) => {
            assert_eq!(p.probabilities, vec![("z".into(), 0.8), ("a".into(), 0.2)])
        }
        event => panic!("Unexpected {event:?}"),
    }
    assert_eq!(server.worker.join().unwrap(), 1);
    let r = one(response_with_unknown_usage(), Policy::default());
    assert_eq!(
        r.metadata.unwrap().usage,
        Some(Usage {
            input_tokens: None,
            output_tokens: None
        })
    );
}
fn response_with_unknown_usage() -> String {
    let mut body = response();
    body["usage"] = json!({"input_tokens":null});
    body.to_string()
}

fn predicate_request() -> PredicateRequest {
    PredicateRequest {
        question_id: "category".into(),
        context: "A message".into(),
        instructions: "Is it a greeting?".into(),
        true_description: Some("A greeting".into()),
        false_description: Some("Anything else".into()),
    }
}
fn score_request() -> ScoreRequest {
    ScoreRequest {
        question_id: "category".into(),
        context: "A message".into(),
        instructions: "Rate urgency".into(),
        levels: vec!["low".into(), "medium".into(), "high".into()],
    }
}
fn noul_response(p: f64) -> Value {
    json!({"model":"jev-returned","answers":{"category":{"type":"noul","noul":p}},"usage":{"input_tokens":10,"output_tokens":1}})
}
fn score_response() -> Value {
    json!({"model":"jev-returned","answers":{"category":{"type":"score","score":1.7,"confidence":0.9,"legend":{"2":"high","0":"low","1":"medium"},"probabilities":{"2":0.8,"0":0.1,"1":0.1}}}})
}
#[test]
fn noul_maps_native_probability_and_boolean_policy_without_confidence() {
    for (p, expected) in [
        (0.98, Decision::Accepted(true)),
        (0.02, Decision::Accepted(false)),
        (0.5, Decision::Abstained(Reason::PolicyRejected)),
    ] {
        let server = mock(
            200,
            noul_response(p).to_string(),
            "X-TypeSafe-Request-Id: noul-42\r\n",
            Duration::ZERO,
            false,
        );
        let mut client = JevClient::new(config(&server.endpoint)).unwrap();
        let report = client.decide_noul(
            &predicate_request(),
            &PredicatePolicy::default(),
            Observation::default,
        );
        assert_eq!(report.core.decision, expected);
        assert_eq!(report.core.rules.len(), 9);
        assert_eq!(report.adapter_error, None);
        assert!(
            matches!(report.prediction, Some(JevPrediction::Scalar(ScalarPrediction::Predicate {probability_true,..})) if probability_true == p)
        );
        assert_eq!(
            report.metadata.unwrap().typesafe_request_id.as_deref(),
            Some("noul-42")
        );
        let (_, payload) = server.seen.recv().unwrap();
        assert_eq!(
            payload,
            json!({"model":"jev-requested","state":"A message","questions":{"category":{"type":"noul","instructions":"Is it a greeting?","criteria":{"true":"A greeting","false":"Anything else"}}}})
        );
        assert_eq!(server.worker.join().unwrap(), 1);
    }
}
#[test]
fn score_maps_ordered_rubric_and_native_expected_score() {
    let server = mock(
        200,
        score_response().to_string(),
        "X-Request-Id: score-gateway\r\n",
        Duration::ZERO,
        false,
    );
    let mut client = JevClient::new(config(&server.endpoint)).unwrap();
    let report = client.decide_score(
        &score_request(),
        &ScorePolicy {
            min_score: Some(1.5),
            min_confidence: Some(0.8),
            ..Default::default()
        },
        Observation::default,
    );
    assert_eq!(report.core.decision, Decision::Accepted(1.7));
    assert_eq!(report.core.rules.len(), 9);
    assert_eq!(
        report.metadata.unwrap().gateway_request_id.as_deref(),
        Some("score-gateway")
    );
    let (_, payload) = server.seen.recv().unwrap();
    assert_eq!(
        payload,
        json!({"model":"jev-requested","state":"A message","questions":{"category":{"type":"score","instructions":"Rate urgency","criteria":["low","medium","high"]}}})
    );
    assert_eq!(server.worker.join().unwrap(), 1);
}
#[test]
fn scalar_wrong_kinds_missing_values_and_invalid_legends_fail_closed() {
    let mut cases = vec![];
    for kind in 0..4 {
        let mut body = noul_response(0.98);
        match kind {
            0 => {
                body["answers"]["category"]["noul"] = "0.98".into();
            }
            1 => {
                body["answers"]["category"]["type"] = "predicate".into();
            }
            2 => {
                body["answers"]["category"]
                    .as_object_mut()
                    .unwrap()
                    .remove("noul");
            }
            _ => {
                body["answers"] = json!({"wrong":body["answers"]["category"].clone()});
            }
        }
        cases.push((
            ScalarRequest::Predicate(predicate_request()),
            body.to_string(),
        ));
    }
    for kind in 0..7 {
        let mut body = score_response();
        match kind {
            0 => {
                body["answers"]["category"]
                    .as_object_mut()
                    .unwrap()
                    .remove("confidence");
            }
            1 => {
                body["answers"]["category"]["legend"]["0"] = "wrong".into();
            }
            2 => {
                body["answers"]["category"]["legend"]["3"] = "extra".into();
            }
            3 => {
                body["answers"]["category"]["probabilities"] = json!({"0":0.1,"1":0.1,"3":0.8});
            }
            4 => {
                body["answers"]["category"]["score"] = "1.7".into();
            }
            5 => {
                body["answers"]["extra"] = body["answers"]["category"].clone();
            }
            _ => {
                body["answers"]["category"]["probabilities"]["0"] = false.into();
            }
        }
        cases.push((ScalarRequest::Score(score_request()), body.to_string()));
    }
    cases.push((ScalarRequest::Score(score_request()),r#"{"model":"jev","answers":{"category":{"type":"score","score":1.7,"confidence":0.9,"legend":{"0":"low","0":"low","1":"medium","2":"high"},"probabilities":{"0":0.1,"1":0.1,"2":0.8}}}}"#.into()));
    cases.push((
        ScalarRequest::Predicate(predicate_request()),
        r#"{"model":"jev","answers":{"category":{"type":"noul","noul":0.9,"noul":0.98}}}"#.into(),
    ));
    for (request, body) in cases {
        let server = mock(200, body, "", Duration::ZERO, false);
        let mut client = JevClient::new(config(&server.endpoint)).unwrap();
        assert!(matches!(
            client.invoke_scalar(&request),
            ScalarEvent::Failed(BackendFailure::MalformedResponse)
        ));
        assert_eq!(server.worker.join().unwrap(), 1);
    }
}
#[test]
fn scalar_numeric_errors_reach_core_specifications() {
    for p in [-0.1, 1.1] {
        let server = mock(200, noul_response(p).to_string(), "", Duration::ZERO, false);
        let mut client = JevClient::new(config(&server.endpoint)).unwrap();
        let r = client.decide_predicate(
            &predicate_request(),
            &PredicatePolicy::default(),
            Observation::default,
        );
        assert_eq!(r.core.decision, Decision::Failed(Failure::OutputValidation));
        assert_eq!(
            r.core.rules.last().unwrap().rule,
            "ScalarProbabilitiesValid"
        );
        assert_eq!(r.adapter_error, None);
        assert_eq!(server.worker.join().unwrap(), 1);
    }
    for (field, v, rule) in [
        ("score", 1.8, "ScalarValueConsistent"),
        ("confidence", -0.1, "ScalarConfidenceValid"),
    ] {
        let mut body = score_response();
        body["answers"]["category"][field] = v.into();
        let server = mock(200, body.to_string(), "", Duration::ZERO, false);
        let r = JevClient::new(config(&server.endpoint))
            .unwrap()
            .decide_score(
                &score_request(),
                &ScorePolicy::default(),
                Observation::default,
            );
        assert_eq!(r.core.decision, Decision::Failed(Failure::OutputValidation));
        assert_eq!(r.core.rules.last().unwrap().rule, rule);
        assert_eq!(r.adapter_error, None);
        assert!(r.metadata.is_some());
        assert_eq!(server.worker.join().unwrap(), 1);
    }
}
#[test]
fn scalar_operational_failures_do_not_select_fallback() {
    for (status, failure) in [
        (401, Failure::Authentication),
        (422, Failure::UnsupportedCapability),
        (504, Failure::TimedOut),
    ] {
        let server = mock(
            status,
            "secret error body".into(),
            "",
            Duration::ZERO,
            false,
        );
        let p = PredicatePolicy {
            fallback: Some(ValueFallback {
                value: false,
                triggers: vec![Reason::PolicyRejected, Reason::ProviderRefusal],
            }),
            ..Default::default()
        };
        let r = JevClient::new(config(&server.endpoint))
            .unwrap()
            .decide_noul(&predicate_request(), &p, Observation::default);
        assert_eq!(r.core.decision, Decision::Failed(failure));
        assert_eq!(r.adapter_error, Some(Error::Http(status)));
        assert!(r.metadata.is_none());
        assert_eq!(server.worker.join().unwrap(), 1);
    }
}
#[test]
fn scalar_size_limits_and_json_state_reject_before_connecting() {
    for request in [
        ScalarRequest::Predicate(predicate_request()),
        ScalarRequest::Score(score_request()),
    ] {
        let mut c = config("http://127.0.0.1:1/");
        c.max_request_bytes = 80;
        let mut client = JevClient::new(c).unwrap();
        assert!(matches!(
            client.invoke_scalar(&request),
            ScalarEvent::Failed(BackendFailure::UnsupportedCapability)
        ));
    }
    let mut c = config("http://127.0.0.1:1/");
    c.state_encoding = StateEncoding::Json;
    let mut r = predicate_request();
    r.context = r#"{"text":1,"text":2}"#.into();
    let report = JevClient::new(c).unwrap().decide_noul(
        &r,
        &PredicatePolicy::default(),
        Observation::default,
    );
    assert_eq!(report.adapter_error, Some(Error::InvalidRequest));
}
#[test]
fn scalar_json_state_and_absent_predicate_criteria_are_explicit() {
    let server = mock(
        200,
        noul_response(0.98).to_string(),
        "",
        Duration::ZERO,
        false,
    );
    let mut c = config(&server.endpoint);
    c.state_encoding = StateEncoding::Json;
    let mut r = predicate_request();
    r.context = r#"{"message":"Hello"}"#.into();
    r.true_description = None;
    r.false_description = None;
    let report = JevClient::new(c).unwrap().decide_predicate(
        &r,
        &PredicatePolicy::default(),
        Observation::default,
    );
    assert_eq!(report.core.decision, Decision::Accepted(true));
    let (_, body) = server.seen.recv().unwrap();
    assert_eq!(body["state"], json!({"message":"Hello"}));
    assert!(body["questions"]["category"].get("criteria").is_none());
    assert_eq!(server.worker.join().unwrap(), 1);
}
#[test]
fn metadata_does_not_leak_between_scalar_and_choice_operations() {
    let server = mock(200, score_response().to_string(), "", Duration::ZERO, false);
    let mut client = JevClient::new(config(&server.endpoint)).unwrap();
    let r = client.decide_score(
        &score_request(),
        &ScorePolicy::default(),
        Observation::default,
    );
    assert!(r.metadata.is_some());
    let cancelled = || Observation {
        cancelled: true,
        expired: false,
    };
    let r = client.decide_noul(&predicate_request(), &PredicatePolicy::default(), cancelled);
    assert_eq!(r.core.decision, Decision::Cancelled);
    assert!(r.metadata.is_none());
    assert!(r.adapter_error.is_none());
    assert!(r.prediction.is_none());
    let r = client.decide(&request(), &Policy::default(), cancelled);
    assert_eq!(r.core.decision, Decision::Cancelled);
    assert!(r.metadata.is_none());
    assert!(r.adapter_error.is_none());
    assert!(r.prediction.is_none());
    assert_eq!(server.worker.join().unwrap(), 1);
}
#[test]
fn scalar_response_quota_and_timeout_use_shared_transport() {
    let server = mock(200, score_response().to_string(), "", Duration::ZERO, true);
    let mut c = config(&server.endpoint);
    c.max_response_bytes = 20;
    let r = JevClient::new(c).unwrap().decide_score(
        &score_request(),
        &ScorePolicy::default(),
        Observation::default,
    );
    assert_eq!(r.adapter_error, Some(Error::ResponseTooLarge));
    assert_eq!(server.worker.join().unwrap(), 1);
    let server = mock(
        200,
        noul_response(0.98).to_string(),
        "",
        Duration::from_millis(800),
        false,
    );
    let mut c = config(&server.endpoint);
    c.timeout = Duration::from_millis(200);
    c.connect_timeout = c.timeout;
    let r = JevClient::new(c).unwrap().decide_noul(
        &predicate_request(),
        &PredicatePolicy::default(),
        Observation::default,
    );
    assert_eq!(r.core.decision, Decision::Failed(Failure::TimedOut));
    assert_eq!(r.adapter_error, Some(Error::TimedOut));
    assert_eq!(server.worker.join().unwrap(), 1);
}
