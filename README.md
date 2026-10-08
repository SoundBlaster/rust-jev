# rust-jev

Synchronous TypeSafe Jev Choice HTTP adapter for
[RustDecision](https://github.com/SoundBlaster/rust-decision). Credentials,
transport, native wire types and provenance stay here; numerical validation,
local enum mapping and acceptance policy stay in RustDecision.

```rust
use rust_jev::{Config, JevClient, rust_decision::{Observation, Policy, Request}};
// Supply credentials from your application's secret store, never source code.
// let mut client = JevClient::new(Config::new(&api_key, model)?)?;
// let report = client.decide(&request, &Policy::default(), Observation::default);
// report.core.decision distinguishes Accepted / Abstained / Fallback / Failed / Cancelled.
```

## Configuration

`Config::new(key, model)` requires an explicit model name. The default endpoint
is `https://api.typesafe.ai/v1/systemone`; `endpoint` is a **full endpoint URL**,
including a gateway prefix and `/v1/systemone`, not a base URL. Configure a proxy
by changing that field and `provider_label` explicitly. Keys do not implicitly
fall back to another provider and environment proxy settings are ignored.

Default HTTP timeout is 10 seconds, connect timeout 5 seconds; both must be
positive and connect timeout must not exceed HTTP timeout. Configured HTTP
limits are at most 300 seconds and 16 MiB per request/response, with 1 MiB body
limits by default. Wire byte quota is enforced before send and while reading,
with or without Content-Length. Transport timeout includes response-body reading,
not serialization or all CPU/core work. No retries, redirects or automatic
compression are enabled. HTTPS is required; the explicit `allow_loopback_http()`
exception accepts only literal loopback IPs for local mocks.

`StateEncoding::Text` sends `Request.context` as a JSON string. Explicit
`StateEncoding::Json` parses it as a JSON string/object/array, preserving the
current object-state consumer contract when supplied with serialized JSON.
There is no automatic JSON detection. Duplicate keys are rejected everywhere.
Caller option order is preserved in criteria; response probabilities are
reconciled by ID to that order. The initial adapter accepts string descriptions
and a single Choice question, with a local maximum of 255 options.

## Outcomes and evidence

Use `JevClient::decide` for an invocation-local snapshot: core decision/rule trace,
requested/returned model IDs, optional usage, adapter version and separate
`x-typesafe-request-id` / gateway `x-request-id`. Missing counts remain unknown.
Numeric-but-invalid confidence/probabilities reach named core validation rules;
wrong wire types, absent native confidence, duplicate/mismatched answers and
option keys fail as malformed response. Additional non-conflicting response
fields are tolerated. Usage absence/null is supported for compatibility with
Jev4Mellea and gateways; the current TypeSafe OpenAPI declares it required.

HTTP 401/403 map to authentication failure; 408/504 to timeout; 422 and local
unsupported request/size failures to unsupported capability; other HTTP failures
(including redirects and rate limiting) to transport failure. The adapter error
also retains the safe status code. Oversized responses are malformed response.
Error/Config/Client Debug representations contain no credentials or response
bodies. The adapter performs no logging; reports contain model/request metadata
and local application values, so applications own their logging policy.

The current native Choice schema has no explicit refusal variant. Unknown answer
types fail closed; no OpenAI refusal shape is invented. Noul, Score, batch,
model discovery and OpenAI Decisions remain separate future contracts.

Use this blocking client on a synchronous thread, including construction/drop.
Boundary observations can cancel before HTTP and on return, but cannot interrupt
an in-flight blocking call. HTTP timeout bounds transport; no async cancellation
capability is advertised. One backend invocation does not guarantee one network
request: locally rejected wire requests perform zero HTTP sends.

## Validation

```sh
cargo test --locked
cargo +1.85.0 test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Tests use real loopback HTTP fixtures, not billable inference. They verify wire
mapping, identity and duplicates, numerical delegation, HTTP categories,
redirect/retry policy, timeouts before headers and during body reads, quotas,
unknown usage and clearing metadata on skipped invocation. CI uses stable and
Rust 1.85 with a committed lockfile and full Git SHA for RustDecision.

For one explicitly opted-in, potentially billable smoke request:

```sh
# Set TYPESAFE_API_KEY and TYPESAFE_MODEL in your environment/secret manager.
cargo run --locked --example live_choice -- --live
# For a gateway, additionally set JEV_ENDPOINT to its full endpoint URL.
```

No live requests are made by tests or builds. The probe prints safe decision
categories and provenance, not prompts, response bodies or credentials; it does
not assert semantic classification accuracy.

See [the adapter contract](docs/adapter-contract.md) and
[native mapping evidence](docs/native-choice-wire.md).
