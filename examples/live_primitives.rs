//! Opt-in live probe: exactly one Noul and one Score request, potentially billable.
use rust_jev::{rust_decision::*, *};
fn summary<T: std::fmt::Debug>(kind: &str, r: JevReport<T>) -> bool {
    let evidence = match r.prediction {
        Some(JevPrediction::Scalar(ScalarPrediction::Predicate {
            probability_true, ..
        })) => serde_json::json!({"probability_true":probability_true}),
        Some(JevPrediction::Scalar(ScalarPrediction::Score {
            score,
            probabilities,
            confidence,
            ..
        })) => {
            serde_json::json!({"score":score,"probabilities":probabilities,"confidence":format!("{confidence:?}")})
        }
        _ => serde_json::Value::Null,
    };
    println!(
        "{}",
        serde_json::json!({"kind":kind,"decision":format!("{:?}",r.core.decision),"invocations":r.core.invocations,"rules":r.core.rules.len(),"evidence":evidence,"adapter_error":r.adapter_error.map(|e|e.to_string()),"metadata":r.metadata.map(|m|serde_json::json!({"provider":m.provider_label,"requested_model":m.requested_model,"returned_model":m.returned_model,"typesafe_request_id":m.typesafe_request_id,"gateway_request_id":m.gateway_request_id,"usage":m.usage.map(|u|serde_json::json!({"input_tokens":u.input_tokens,"output_tokens":u.output_tokens}))}))})
    );
    matches!(r.core.decision, Decision::Accepted(_))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--live"] {
        return Err("Pass --live to authorize two potentially billable requests".into());
    }
    let key = std::env::var("TYPESAFE_API_KEY").map_err(|_| "Missing TYPESAFE_API_KEY")?;
    let model = std::env::var("TYPESAFE_MODEL").map_err(|_| "Missing TYPESAFE_MODEL")?;
    let mut config = Config::new(&key, model)?;
    if let Ok(endpoint) = std::env::var("JEV_ENDPOINT") {
        config.endpoint = endpoint;
        config.provider_label = "typesafe-proxy".into();
    }
    let mut client = JevClient::new(config)?;
    let noul = PredicateRequest {
        question_id: "greeting".into(),
        context: "Hello, good morning!".into(),
        instructions: "Does the message contain a greeting?".into(),
        true_description: Some("A greeting is present".into()),
        false_description: Some("No greeting is present".into()),
    };
    let score = ScoreRequest {
        question_id: "politeness".into(),
        context: "Hello, good morning! Thank you for your help.".into(),
        instructions: "Rate how polite the message is.".into(),
        levels: vec!["Impolite".into(), "Neutral".into(), "Polite".into()],
    };
    let first = summary(
        "noul",
        client.decide_noul(&noul, &PredicatePolicy::default(), Observation::default),
    );
    let second = summary(
        "score",
        client.decide_score(&score, &ScorePolicy::default(), Observation::default),
    );
    if !first || !second {
        return Err("A primitive smoke did not reach acceptance".into());
    }
    Ok(())
}
