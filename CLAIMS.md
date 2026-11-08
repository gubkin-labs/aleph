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
| No app can take the CPU other apps need | Homepage (loop, cost model), docs: Shared infrastructure | Operations are bounded and metered, and the costly ones (`matches`, copies, `in`) are charged for their real work: `tests/usage.rs` runs on every `pnpm verify`. `benches/adversarial.rs` measures it: the slowest route that passes `aleph check` takes about 3 ms (about 1.7 µs per op). **Gap:** the timing is a benchmark, not checked on every run; it assumes the platform caps request bodies (about 1 MB) | Partial |
| No app can take the memory other apps need | Homepage, docs: Shared infrastructure | Every value read, copied, or built is charged its in-memory size, statically and by the meter (`tests/usage.rs`, conformance `usage.json` goldens). `benches/adversarial.rs` measures peak heap within each route's bound (at most 4.3 MB). **Gap:** peak heap is measured by a benchmark, not on every run | Partial |
| No request can flood the shared database | Docs: Shared infrastructure | Reads, writes, and bytes read are bounded and metered (rows above) | Backed |
| One app can't read or write another app's data | Homepage, docs: Shared infrastructure | Key encoding property tests in `src/keys.rs` (prefixes, cursors that can't escape their range); `tests/generated_plans.rs` checks no run touches another tenant | Backed |
| Files can't be reached from another app, or beyond what their signed URL allows | Docs: Files | `src/runtime/blobs.rs` tests (tampered tenant, bucket, type, size, expiry, or access is rejected); `conformance/cases/files` (another user's key, wrong type, too large, tampered, expired); file metadata keys come from `Keyspace` | Backed |
| File uploads and deletes keep within their bounds | Docs: Files | `conformance/cases/files` scenario: usage ≤ bounds on every route and the upload handler | Backed |
| The core never panics on any manifest | Docs: Shared infrastructure ("the core never trusts its input") | Validation of mutated conformance manifests in `tests/generated_plans.rs` (every `pnpm verify`); coverage-guided fuzzing in `fuzz/` (`parse_bytes`, `parse_mutated`, `execute`): about 11 CPU-hours and 23M inputs on 2026-11-07 found no panic, overrun, cross-tenant write, or dirty failed request. **Gap:** fuzzing runs by hand, not on every verify | Partial |
| A manifest that passes `aleph check` doesn't fail at runtime from a type the checker could have caught | Implied by the checker (not stated as a promise) | Fuzzing in strict mode found and fixed two (enum and object writes, `conformance/invalid/enum-write.json`, `object-write-extra-field.json`). **Gap:** strict-mode fuzzing still finds 500s for string lengths outside a field's `min`/`max`, null written to a required field, mixed-type ordering, and object-typed key parts, which the checker leaves to the runtime | Partial |

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
| Memory small enough to pack thousands of apps into one process | Homepage (run on Rust) | `benches/load.rs`: 77–215 KB of heap per loaded conformance app, so 10,000 apps take about 1–2 GB. **Gap:** measured on five apps, not a large fleet | Partial |
| A new version loads in milliseconds; no cold starts | Homepage (run on Rust) | `benches/load.rs`: validating and loading a conformance manifest takes 0.12–0.35 ms; 100× the ledger's routes (1.1 MB) takes 14–68 ms. **Gap:** assumes the platform caps manifest size | Partial |
| Requests are microseconds of CPU | Docs: Shared infrastructure | `benches/routes.rs` on the conformance apps. **Gap:** not at the limits | Partial |
| A platform can offer much better pricing than a dedicated deployment | Homepage (safe to share), docs: Shared infrastructure | Follows from sharing; no cost model or measured density yet | Not yet |

## Developer experience

| Claim | Where | Evidence | Status |
|---|---|---|---|
| Agents fix their own mistakes from the checker's diagnostics | Homepage (built for agents) | Diagnostics name the line and the bound. **Gap:** no agent evaluation | Not yet |
| Build a first app in five minutes | Homepage, docs: Quickstart | The quickstart's commands work from npm (checked on 0.1.0 in a fresh project). **Gap:** not timed with new users | Partial |
| A loop over a list that could be too long is rejected before it builds | Homepage (playground: Builtin guardrails) | `website/playground-apps/guardrails.ts` is valid and uses 489 ops; with the list bound raised from 10 to 100, `aleph check` rejects it for 40,809 ops and 12 MB of memory (run by hand against the CLI, not in a test) | Partial |
| A request that reads too much is rejected before it runs | Homepage (playground: Infra protection) | Backed in Guardrails above (`unbounded_limit` / `over_budget` cases). The animation's limit of 250 reads and 120 reads for 20 posts match `aleph check` output for the blog app | Backed |
| An email is queued inside the transaction and sent once, after commit | Homepage (playground: Exactly once by default) | `website/playground-apps/exactly.ts` is valid with one effect; see Correctness for the runtime tests. The Node animation (two separate transactions, a crash between them) illustrates a common pattern and is not run | Partial |
| MCP, auth, and OpenAPI come from the manifest with no extra libraries | Homepage (playground: Platform agnostic) | `aleph openapi` and `aleph mcp` exist in the CLI. **Gap:** auth is declared roles, scopes, and per-route access; sign-in methods (email, OAuth) are not in the manifest yet, so "auth" is partly aspirational | Partial |
| Aleph apps share one runtime and one database, so they cost less than a stack per app | Homepage (playground: Shared infra & reduced costs) | Same as the Performance and cost row on pricing: follows from sharing, with no cost model or measured density yet. The dollar coins are illustrative | Not yet |
| The manifest is executed by a small Rust core, which is faster than a typical language runtime | Homepage (playground: Write in TS, execute in Rust) | Rust core and manifest format exist; see the Performance rows for what is measured (`benches/routes.rs`, `benches/load.rs`). **Gap:** no benchmark compares it with a Node or other runtime; the slower-runtime animation is illustrative | Partial |
| Typical backends flood the database, lose work between steps, and need a library per feature in the ways the Node examples and animations show | Homepage (playground, all tabs) | Illustrative snippets of common patterns (N+1 queries, a nested loop on user input, a write followed by a send, separate libraries). They are not run | Not yet |
