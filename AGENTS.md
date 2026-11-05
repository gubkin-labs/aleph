# Working in this repo

Specs, tickets, and product context live in the sibling `aleph-infra` repo. Read its `PRODUCT.md`
and `spec/architecture.md` before changing the manifest format or the core's guarantees.

## Invariants: don't break these

1. **The Rust types in `crates/aleph-core/src/manifest` are the source of truth** for the format.
   After changing them, run `pnpm codegen`. Never edit `packages/manifest/src/generated*` or
   `schema/` by hand.
2. **Static bounds must cover runtime usage.** Every cost the runtime meter charges (`runtime/exec.rs`)
   must be accounted for by the checker (`check/plan.rs`). The conformance runners and the
   `random_requests_stay_within_static_bounds` property test assert `usage ≤ bounds`. If you add an
   operation, update both sides and add a test.
3. **Keys come only from `Keyspace`.** Plans must never construct raw keys. Tenant isolation depends
   on it.
4. **Handlers are recorded, not run.** SDK code must stay deterministic: no time, randomness, or I/O
   at build time. `app.toJSON()` must produce identical bytes on every build.
5. **The core never trusts its input.** Validation returns all diagnostics, never panics. The
   interpreter re-checks what it can (defense in depth).

## Before you finish

```bash
pnpm fix && cargo fmt --all
```

```bash
pnpm verify
```

There's no hosted CI: `pnpm verify` (`scripts/ci.sh`) is the gate, so run it before every push.

If you intentionally changed SDK output or analysis results, update the golden files with
`pnpm --filter @aleph/conformance update`, then review the diff.

## Conventions

- Rust: edition 2024. `clippy::pedantic` with warnings denied (`pnpm verify`). No `unsafe`. No
  `unwrap`/`expect`/`panic` outside tests. Every public item is documented.
- TypeScript: strict, with `exactOptionalPropertyTypes`, `noUncheckedIndexedAccess`, and
  `isolatedDeclarations`. Biome for lint and format. Vitest for tests. `tsdown` for builds.
- Diagnostics and error messages say what's wrong **and how to fix it**. Agents read them.
- Tests live next to the code they test: Rust unit tests in `#[cfg(test)]` modules, integration
  tests in `crates/*/tests`, and TS tests in `packages/*/test`.
