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
    let state = if json_state {
        let value = strict_json::parse(r.context.as_bytes()).map_err(|_| Error::InvalidRequest)?;
        if !matches!(value, Value::String(_) | Value::Object(_) | Value::Array(_)) {
            return Err(Error::InvalidRequest);
        }
        value
    } else {
        Value::String(r.context.clone())
    };
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
        .get(&r.question_id)
        .and_then(Value::as_object)
        .ok_or(Error::MalformedResponse)?;
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
        Prediction {
            question_id: r.question_id.clone(),
            selected_id: choice,
            probabilities,
            confidence: Confidence::Present(confidence),
        },
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
