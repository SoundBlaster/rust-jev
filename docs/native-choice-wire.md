# Native Choice evidence — 2026-10-08

Verified against the official [TypeSafe OpenAPI](https://api.typesafe.ai/openapi.json)
and [Swagger reference](https://api.typesafe.ai/docs), API version 0.2.0.
Observed OpenAPI SHA-256: `a191f8a7df6bd6fedced8120dd0fd106f88575d1d1c8360d08900a6c7c0360d5`.
This is schema/transport evidence, not a captured inference response.

Request: `POST /v1/systemone`, bearer authorization, JSON `model`, `state` and
named `questions`. `state` is a string/object/array. One Choice question has
`type: choice`, string instructions and an ordered ID-to-description criteria
map. This adapter implements the text-description subset.

Response: nonempty model ID and a named answer map. Choice requires type, choice,
confidence and label-keyed numeric probabilities. The schema includes no refusal
answer. RustDecision validates the numerical domain, normalization, selected
maximum and caller thresholds. Numeric confidence is distinct from selected
probability. Missing native confidence is malformed, not declared unavailable.

The schema requires usage with integer counts. For gateway/Jev4Mellea compatibility
we explicitly permit absent/null usage and counts, retaining unknown as None;
provided counts must be nonnegative integers. Requested and returned model IDs
are separate. Header IDs are independent optional metadata; no revision or
request IDs are fabricated.

State JSON mode is explicit so SpecificationMetrics can retain its object state
in a later migration. The initial core request's context remains provider-neutral
text. Two question axes cannot be migrated as an atomic batch using this single
question API; a separate batch contract is required.

The current Jev4Mellea `providers/typesafe.py` and Choice fixtures informed the
payload and separate request header mapping. Its checkout was read only; no
Python files or configuration were changed.

## Noul and Score extension

Verified against the same official TypeSafe schema. All operations retain the
single named-question envelope, explicit state encoding and transport bounds.

- Noul request: `type: noul`, text instructions, optional true/false text criteria.
  Answer: `type: noul`, numeric `noul` probability. No native confidence field.
  Maps to RustDecision Predicate; native-name aliases are convenience only.
- Score request: `type: score`, text instructions and ordered text criteria array.
  Answer: numeric `score`, required numeric confidence, index-keyed probabilities
  and a legend mapping each index to its exact requested text description.
  Core verifies finite ranges, distribution normalization and weighted-average
  consistency; adapter verifies identities, legend and wire shapes.
- This does not implement OpenAI Predicate wire format or a mixed batch.
