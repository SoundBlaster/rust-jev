//! Explicitly opted-in, potentially billable smoke probe. Never runs in cargo test.
use rust_jev::{rust_decision::*, *};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--live"] {
        return Err("Pass --live to authorize one potentially billable request".into());
    }
    let key = std::env::var("TYPESAFE_API_KEY").map_err(|_| "Missing TYPESAFE_API_KEY")?;
    let model = std::env::var("TYPESAFE_MODEL")
        .map_err(|_| "Missing TYPESAFE_MODEL: select an exact model name")?;
    let mut config = Config::new(&key, model)?;
    if let Ok(endpoint) = std::env::var("JEV_ENDPOINT") {
        config.endpoint = endpoint;
        config.provider_label = "typesafe-proxy".into();
    }
    let request = Request {
        question_id: "kind".into(),
        context: "Hello, good morning!".into(),
        instructions: "Classify the message.".into(),
        options: vec![
            Choice {
                id: "greeting".into(),
                description: "A greeting".into(),
                value: "greeting",
            },
            Choice {
                id: "other".into(),
                description: "Anything else".into(),
                value: "other",
            },
        ],
    };
    let report = JevClient::new(config)?.decide(&request, &Policy::default(), Observation::default);
    println!(
        "{}",
        serde_json::json!({"decision":format!("{:?}",report.core.decision),
        "invocations":report.core.invocations,"evaluated_rules":report.core.rules.len(),
        "adapter_error":report.adapter_error.map(|e|e.to_string()),
        "metadata":report.metadata.map(|m|serde_json::json!({"provider":m.provider_label,
            "requested_model":m.requested_model,"returned_model":m.returned_model,
            "typesafe_request_id":m.typesafe_request_id,"gateway_request_id":m.gateway_request_id,
            "usage":m.usage.map(|u|serde_json::json!({"input_tokens":u.input_tokens,"output_tokens":u.output_tokens}))}))})
    );
    if matches!(
        report.core.decision,
        Decision::Failed(_) | Decision::Cancelled
    ) {
        return Err("Choice smoke did not complete successfully".into());
    }
    Ok(())
}
