use crate::{Error, Metadata, Usage, config::label, strict_json};
use rust_decision::{BackendRequest, Confidence, Prediction};
use serde_json::{Map, Value, json};

pub(crate) fn request(model: &str, r: &BackendRequest, json_state: bool) -> Result<Vec<u8>, Error> {
    if r.question_id.trim().is_empty()
        || r.instructions.trim().is_empty()
        || r.options.is_empty()
        || r.options.len() > 255
        || r.options
            .iter()
            .any(|(id, d)| id.trim().is_empty() || d.trim().is_empty())
    {
        return Err(Error::InvalidRequest);
    }
    let state = state(&r.context, json_state)?;
    let mut criteria = Map::new();
    for (id, description) in &r.options {
        if criteria
            .insert(id.clone(), Value::String(description.clone()))
            .is_some()
        {
            return Err(Error::InvalidRequest);
        }
    }
    let mut questions = Map::new();
    questions.insert(
        r.question_id.clone(),
        json!({"type":"choice", "instructions":r.instructions, "criteria":criteria}),
    );
    serde_json::to_vec(&json!({"model":model,"state":state,"questions":questions}))
        .map_err(|_| Error::InvalidRequest)
}
fn string(value: Option<&Value>) -> Result<String, Error> {
    let s = value
        .and_then(Value::as_str)
        .filter(|s| label(s))
        .ok_or(Error::MalformedResponse)?;
    Ok(s.into())
}
fn count(value: Option<&Value>) -> Result<Option<u64>, Error> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or(Error::MalformedResponse),
    }
}
pub(crate) fn response(
    body: &[u8],
    r: &BackendRequest,
    requested_model: &str,
    provider: &str,
    typesafe_id: Option<String>,
    gateway_id: Option<String>,
) -> Result<(Prediction, Metadata), Error> {
    let (answer, metadata) = envelope(
        body,
        &r.question_id,
        requested_model,
        provider,
        typesafe_id,
        gateway_id,
    )?;
    if answer.get("type").and_then(Value::as_str) != Some("choice") {
        return Err(Error::MalformedResponse);
    }
    let choice = answer
        .get("choice")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(Error::MalformedResponse)?
        .to_owned();
    let confidence = answer
        .get("confidence")
        .and_then(Value::as_f64)
        .ok_or(Error::MalformedResponse)?;
    let probabilities = answer
        .get("probabilities")
        .and_then(Value::as_object)
        .ok_or(Error::MalformedResponse)?;
    if probabilities.len() != r.options.len()
        || r.options
            .iter()
            .any(|(id, _)| !probabilities.contains_key(id))
    {
        return Err(Error::MalformedResponse);
    }
    let probabilities = r
        .options
        .iter()
        .map(|(id, _)| {
            let p = probabilities[id].as_f64().ok_or(Error::MalformedResponse)?;
            Ok((id.clone(), p))
        })
        .collect::<Result<_, Error>>()?;
    Ok((
        Prediction {
            question_id: r.question_id.clone(),
            selected_id: choice,
            probabilities,
            confidence: Confidence::Present(confidence),
        },
        metadata,
    ))
}

fn state(context: &str, json_state: bool) -> Result<Value, Error> {
    let state = if json_state {
        let value = strict_json::parse(context.as_bytes()).map_err(|_| Error::InvalidRequest)?;
        if !matches!(value, Value::String(_) | Value::Object(_) | Value::Array(_)) {
            return Err(Error::InvalidRequest);
        }
        value
    } else {
        Value::String(context.to_owned())
    };
    Ok(state)
}
fn envelope(
    body: &[u8],
    question_id: &str,
    requested_model: &str,
    provider: &str,
    typesafe_id: Option<String>,
    gateway_id: Option<String>,
) -> Result<(Map<String, Value>, Metadata), Error> {
    let root = strict_json::parse(body).map_err(|_| Error::MalformedResponse)?;
    let returned_model = string(root.get("model"))?;
    let answers = root
        .get("answers")
        .and_then(Value::as_object)
        .ok_or(Error::MalformedResponse)?;
    if answers.len() != 1 {
        return Err(Error::MalformedResponse);
    }
    let answer = answers
        .get(question_id)
        .and_then(Value::as_object)
        .ok_or(Error::MalformedResponse)?
        .clone();
    let usage = match root.get("usage") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let usage = value.as_object().ok_or(Error::MalformedResponse)?;
            Some(Usage {
                input_tokens: count(usage.get("input_tokens"))?,
                output_tokens: count(usage.get("output_tokens"))?,
            })
        }
    };
    Ok((
        answer,
        Metadata {
            provider_label: provider.into(),
            requested_model: requested_model.into(),
            returned_model,
            typesafe_request_id: typesafe_id,
            gateway_request_id: gateway_id,
            usage,
            adapter_version: env!("CARGO_PKG_VERSION"),
        },
    ))
}

pub(crate) fn scalar_request(
    model: &str,
    r: &rust_decision::ScalarRequest,
    json_state: bool,
) -> Result<Vec<u8>, Error> {
    use rust_decision::ScalarRequest;
    if r.question_id().trim().is_empty() || r.instructions().trim().is_empty() {
        return Err(Error::InvalidRequest);
    }
    let question = match r {
        ScalarRequest::Predicate(r) => {
            let mut criteria = Map::new();
            for (key, description) in [
                ("true", &r.true_description),
                ("false", &r.false_description),
            ] {
                if let Some(description) = description {
                    if description.trim().is_empty() {
                        return Err(Error::InvalidRequest);
                    }
                    criteria.insert(key.into(), Value::String(description.clone()));
                }
            }
            let mut q = json!({"type":"noul", "instructions":r.instructions});
            if !criteria.is_empty() {
                q["criteria"] = Value::Object(criteria);
            }
            q
        }
        ScalarRequest::Score(r) => {
            if r.levels.is_empty()
                || r.levels.len() > 255
                || r.levels.iter().any(|s| s.trim().is_empty())
            {
                return Err(Error::InvalidRequest);
            }
            json!({"type":"score", "instructions":r.instructions, "criteria":r.levels})
        }
    };
    let mut questions = Map::new();
    questions.insert(r.question_id().into(), question);
    serde_json::to_vec(
        &json!({"model":model,"state":state(r.context(), json_state)?,"questions":questions}),
    )
    .map_err(|_| Error::InvalidRequest)
}
pub(crate) fn scalar_response(
    body: &[u8],
    r: &rust_decision::ScalarRequest,
    requested_model: &str,
    provider: &str,
    typesafe_id: Option<String>,
    gateway_id: Option<String>,
) -> Result<(rust_decision::ScalarPrediction, Metadata), Error> {
    use rust_decision::{ScalarPrediction, ScalarRequest};
    let (answer, metadata) = envelope(
        body,
        r.question_id(),
        requested_model,
        provider,
        typesafe_id,
        gateway_id,
    )?;
    let number = |key: &str| {
        answer
            .get(key)
            .and_then(Value::as_f64)
            .ok_or(Error::MalformedResponse)
    };
    let prediction = match r {
        ScalarRequest::Predicate(r) => {
            if answer.get("type").and_then(Value::as_str) != Some("noul") {
                return Err(Error::MalformedResponse);
            }
            ScalarPrediction::Predicate {
                question_id: r.question_id.clone(),
                probability_true: number("noul")?,
            }
        }
        ScalarRequest::Score(r) => {
            if answer.get("type").and_then(Value::as_str) != Some("score") {
                return Err(Error::MalformedResponse);
            }
            let legend = answer
                .get("legend")
                .and_then(Value::as_object)
                .ok_or(Error::MalformedResponse)?;
            let probabilities = answer
                .get("probabilities")
                .and_then(Value::as_object)
                .ok_or(Error::MalformedResponse)?;
            if legend.len() != r.levels.len() || probabilities.len() != r.levels.len() {
                return Err(Error::MalformedResponse);
            }
            let probabilities = r
                .levels
                .iter()
                .enumerate()
                .map(|(i, description)| {
                    let key = i.to_string();
                    if legend.get(&key).and_then(Value::as_str) != Some(description.as_str()) {
                        return Err(Error::MalformedResponse);
                    }
                    let value = probabilities
                        .get(&key)
                        .and_then(Value::as_f64)
                        .ok_or(Error::MalformedResponse)?;
                    Ok((i, value))
                })
                .collect::<Result<_, Error>>()?;
            ScalarPrediction::Score {
                question_id: r.question_id.clone(),
                score: number("score")?,
                probabilities,
                confidence: Confidence::Present(number("confidence")?),
            }
        }
    };
    Ok((prediction, metadata))
}
