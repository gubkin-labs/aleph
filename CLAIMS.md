# Claims ledger

Every promise the homepage, docs, and README make about Aleph, and the evidence that it holds.
A claim without evidence gets a test or measurement, or comes off the site. When you add a public
claim, add a row here in the same change.

**Status:** **Backed**: a test or measurement in this repo shows it, and runs on every
`pnpm verify`. **Partial**: part of it is shown; the gap is named. **Not yet**: stated as
intent or design; nothing here proves it yet.

## Guardrails

| Claim | Where | Evidence | Status |
|---|---|---|---|
| Every route and handler has a worst case on 7 resources, computed before deploy | Homepage (usage limits), docs: Bounded by construction | `crates/aleph-core/src/check/plan.rs`; golden `conformance/cases/*/bounds.json` | Backed |
| No query can read too much; an unbounded query doesn't build | Homepage (guardrails) | `unbounded_limit` / `over_budget` cases in `conformance/invalid/` | Backed |
| A runtime meter enforces the same limits, and never trips for a valid plan | Homepage, docs: Bounded by construction | `tests/generated_plans.rs` (100,000 random plans on every `pnpm verify`: usage ≤ bounds), conformance `scenarios.json` (usage ≤ bounds per step) | Backed |
| No loop can run away: every loop has a known maximum; no recursion | Homepage, docs: How Aleph programs work | `unbounded_list` checks; `$.loop` `max` and state-growth checks (`conformance/invalid/loop-errors.json`) | Backed |
| Handlers can't set each other off without end | Docs: Background work | `src/check/cycles.rs`, `conformance/invalid/trigger-cycle.json`, `tests/runtime.rs` (`a_trigger_cycle_stops_at_its_declared_depth`) | Backed |
| No app can take the CPU other apps need | Homepage (loop, cost model), docs: Shared infrastructure | Operations are bounded and metered. **Gap:** the meter counts operations, not time; the worst real CPU time of a route at the limits isn't measured yet | Partial |
| No app can take the memory other apps need | Homepage, docs: Shared infrastructure | The meter charges each value's size. **Gap:** real allocation against the charged bytes isn't measured | Partial |
| No request can flood the shared database | Docs: Shared infrastructure | Reads, writes, and bytes read are bounded and metered (rows above) | Backed |
| One app can't read or write another app's data | Homepage, docs: Shared infrastructure | Key encoding property tests in `src/keys.rs` (prefixes, cursors that can't escape their range); `tests/generated_plans.rs` checks no run touches another tenant | Backed |
| Files can't be reached from another app, or beyond what their signed URL allows | Docs: Files | `src/runtime/blobs.rs` tests (tampered tenant, bucket, type, size, expiry, or access is rejected); `conformance/cases/files` (another user's key, wrong type, too large, tampered, expired); file metadata keys come from `Keyspace` | Backed |
| File uploads and deletes keep within their bounds | Docs: Files | `conformance/cases/files` scenario: usage ≤ bounds on every route and the upload handler | Backed |
| The core never panics on any manifest | Docs: Shared infrastructure ("the core never trusts its input") | Validation of mutated conformance manifests in `tests/generated_plans.rs`. **Gap:** no coverage-guided fuzzing yet | Partial |

## Correctness

| Claim | Where | Evidence | Status |
|---|---|---|---|
| Every request is one serializable transaction; conflicts re-run | Homepage (correct by default), docs: Transactions | `tests/runtime.rs` (`concurrent_deposits_serialize_with_conflict_retries`), `src/store/memory.rs` conflict tests | Backed (in-memory store) |
| Effects go out exactly once, only after commit | Homepage, docs: Transactions | `tests/runtime.rs` (`effects_are_released_exactly_once_under_conflict_retries`). **Gap:** no crash or fault-injection tests | Partial |
| Idempotent routes replay their response for a repeated key | Docs: Routes | `tests/runtime.rs` (`concurrent_retries_of_one_idempotent_transfer_apply_once`) | Backed |
| Breaking schema changes are blocked until deprecated | Homepage (change it without fear), docs: Transactions | Evolution checks in `tests/conformance.rs` | Backed |
| `matches` runs in linear time | README | Uses `regex-lite`, which doesn't backtrack | Backed (by dependency) |
| `aleph dev` and `aleph test` run exactly what production runs | Homepage, docs | The same core, compiled to WebAssembly. **Gap:** no production runtime exists yet to compare against | Partial |

## Performance and cost

| Claim | Where | Evidence | Status |
|---|---|---|---|
| No garbage-collection pauses, steady latency under load | Homepage (run on Rust) | The core is Rust; `benches/routes.rs` times single requests. **Gap:** no load test | Partial |
| Memory small enough to pack thousands of apps into one process | Homepage (run on Rust) | Not measured | Not yet |
| A new version loads in milliseconds; no cold starts | Homepage (run on Rust) | Not measured | Not yet |
| Requests are microseconds of CPU | Docs: Shared infrastructure | `benches/routes.rs` on the conformance apps. **Gap:** not at the limits | Partial |
| A platform can offer much better pricing than a dedicated deployment | Homepage (safe to share), docs: Shared infrastructure | Follows from sharing; no cost model or measured density yet | Not yet |

## Developer experience

| Claim | Where | Evidence | Status |
|---|---|---|---|
| Agents fix their own mistakes from the checker's diagnostics | Homepage (built for agents) | Diagnostics name the line and the bound. **Gap:** no agent evaluation | Not yet |
| Build a first app in five minutes | Homepage, docs: Quickstart | The quickstart's commands work from npm (checked on 0.1.0 in a fresh project). **Gap:** not timed with new users | Partial |
