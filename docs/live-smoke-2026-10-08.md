# Live native primitive smoke — 2026-10-08 UTC

Opt-in direct TypeSafe requests through RustJev, using a process-environment key.
No key, error bodies or endpoint credential material is stored here.

## Implementation and provenance

- Existing Choice probe: merged adapter commit `6859034`, 21:14 UTC.
- New Noul/Score probe: feature implementation over `6859034`, 21:33 UTC;
  core pinned to `6964ab79a25f98ca99f7e3c1d603394e72cd2677` (RustDecision PR #6).
- Requested model: `jev-latest`; returned model for each request: `jev-1.13.0`.
- Choice used `examples/live_choice.rs`; Noul and Score used `examples/live_primitives.rs`.
- Each primitive made one inference invocation and passed nine core rules.
- Keys came from the existing local environment; automatic retries were disabled.

## Results

| Primitive | Example | Decision | Native evidence | Token usage input/output |
| --- | --- | --- | --- | --- |
| Choice | Greeting classification | Accepted(greeting) | Existing probe did not retain native probabilities | 315 / 35 |
| Noul | Whether a greeting is present | Accepted(true) | p_true = 0.99 | 302 / 22 |
| Score | Politeness on a three-level rubric | Accepted(2.0) | p(0)=0, p(1)=0, p(2)=1; confidence=1 | 311 / 19 |

Choice took 3.051 seconds. The two new primitive requests together took 3.192
seconds including client creation. These are single observations, not latency
percentiles. Score used default absolute epsilon 1e-6; its value and distribution
agreed without rounding or repair.

## Evidence boundary

The live probes verify native transport, response mapping and concrete core
validation paths. They do not establish semantic accuracy or threshold
calibration. Choice live evidence predates the shared transport refactoring;
23 HTTP regressions cover that refactoring and all three kinds.
The CoreInfra proxy has not been run in this task: its separate credential is
not available. This evidence does not claim OpenAI wire or mixed-batch support.
