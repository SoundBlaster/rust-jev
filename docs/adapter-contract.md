# RustJev adapter contract proposal

Status: theoretical preparation, 2026-10-07. No client is implemented by this
document. Package name: `rust-jev`; Rust import: `rust_jev`.

## Responsibility and dependency

RustJev owns TypeSafe Jev HTTP transport, credentials, native request/response
types and conversion to the proposed
[RustDecision contract](https://github.com/SoundBlaster/rust-decision).
RustDecision owns typed application mapping, validation Specifications,
acceptance, abstention and fallback. Consumer-specific code classification,
intent, evidence and metrics remain in SpecificationMetrics.

RustJev depends on RustDecision. It does not implement OpenAI Decisions: a
future OpenAI adapter targets RustDecision independently.

## Starting evidence and Choice mapping

The existing SpecificationMetrics client is a bounded extraction candidate,
not a complete Jev SDK. Inspected source:
[classify.rs](https://github.com/SoundBlaster/SpecificationMetrics/blob/169ebfd3921e1cdffc9e43b634e66b6b174aabee/src/classify.rs).
It sends model, state and a map of named questions. Choice questions contain
instructions and a criteria map. Responses expose a map of named answers,
label-keyed probabilities, choice, confidence, returned model and usage.
These observations describe the current consumer adapter; check the current
vendor contract before implementing additional primitives or limits.

Map core opaque string option IDs to Jev criteria keys, retain descriptions and
instructions, and reconcile response keys to request ordering. Validate exact
question/option identities and response kinds. Retain native confidence with
its provider provenance. Do not invent explanations or immutable model revision
IDs when the provider does not supply them. Absent metadata remains absent.

The core's initial request is bounded text. The consumer supplies its serialized
context explicitly; the adapter must document how that text becomes Jev state
and verify parity against the current object-state consumer before migration.
Do not introduce provider JSON types into the core to avoid this mapping task.

## Operational boundary

Configure endpoint, model and credentials explicitly. Validate the endpoint and
define local-mock exceptions separately. Never log tokens; safe Debug and error
representations must redact them. Bound time and request/response bytes, disable
redirects and automatic retries initially. Exact limits are adapter settings
subject to vendor constraints, not copied into the generic decision contract.

Support explicit refusal if the verified Jev contract supplies it; do not
invent a Jev refusal shape from OpenAI documentation. Preserve distinct
transport, authentication, malformed-output, timeout and cancellation failures.
Operational failure is not a low-confidence prediction. An async implementation
must establish cancellation/deadline semantics before advertising them.

Native multi-question requests may preserve the current consumer's two-axis
call in a later batch contract. Initially do not claim two separate Choice
calls are one atomic request. Migration must explicitly account for request
counts, budgets, partial failures and report provenance.

## API differences that must remain in adapters

| Aspect | Existing Jev consumer mapping | OpenAI Decisions documented mapping |
| --- | --- | --- |
| Input | state | input |
| Questions | Named map | Array with names |
| Choice options | criteria map | choices array |
| Answers | Named map | Array with names |
| Distribution | Label-keyed map | Per-value entries |
| Boolean primitive | Noul terminology | Predicate terminology |
| Refusal | Verify vendor support | Explicit refusal answer |

OpenAI facts were checked 2026-10-07 in the official
[Decisions reference](https://developers.openai.com/api/reference/resources/decisions/methods/create).
The mapping is a design comparison, not a promise of interchangeable scores,
calibration, SDK support or account access. Keep provider-selected labels and
score semantics intact; never adapt by changing only the endpoint URL.

Coordination: [Jev4Mellea PR #28](https://github.com/SoundBlaster/Jev4Mellea/pull/28),
inspected at `7185527da2430219c454d33bc792e093b3a14e80`, plans OpenAI integration.
Its proposal must not be represented as completed support in Rust or Python.

## Delivery and validation

1. Agree the RustDecision boundary and current vendor Choice mapping.
2. Implement explicit client configuration, native types and Choice adapter.
3. Use local mock transport for success, named identity mismatch, missing and
   extra answers, malformed data, oversized bodies, authorization failures,
   timeouts and any verified refusal representation.
4. Add separately opted-in live smoke tests without checked-in credentials.
   Record transport compatibility separately from semantic accuracy.
5. Migrate SpecificationMetrics with before/after artifact parity, preserving
   opportunity and concern_kind independently, review authority and S/U.
   Design batch semantics first if required for current two-question parity.

Noul, Score and additional execution modes need follow-up contracts. Confidence
thresholds are caller policies evaluated on labeled data; RustJev supplies no
universal threshold or automatic fallback provider.
