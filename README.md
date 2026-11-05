# Aleph

Aleph is a declarative language for backends. You describe routes, data, and logic in TypeScript.
The SDK compiles that into a versioned **manifest**, and a platform runs the manifest on shared
infrastructure. Every operation is bounded, so thousands of apps can share the same processes and
databases safely.

This repository is the open-source half of Aleph:

| Path | What it is |
|---|---|
| [`crates/aleph-core`](crates/aleph-core) | The trusted core (Rust): manifest types, validator, static cost analyzer, bounded plan interpreter, runtime budget meter, tenant-isolated key encoding, and an in-memory transactional store |
| [`crates/aleph-core-wasm`](crates/aleph-core-wasm) | `aleph-core` compiled to WASM |
| [`crates/xtask`](crates/xtask) | Code generation: JSON Schema and TypeScript types from the Rust types |
| [`packages/manifest`](packages/manifest) | `@aleph/manifest`: generated TypeScript types for the manifest format |
| [`packages/core-wasm`](packages/core-wasm) | `@aleph/core`: validation, cost analysis, and a local dev engine (the Rust core, via WASM) |
| [`packages/sdk`](packages/sdk) | `@aleph/sdk`: the TypeScript builder that produces manifests |
| [`conformance`](conformance) | Golden apps, manifests, bounds, and request scenarios that the SDK, WASM, and native core must all agree on |
| [`schema`](schema) | `manifest.schema.json`, for third-party SDKs and platforms |

## A taste

```ts
import { app, err, t } from "@aleph/sdk";

const a = app("projects");

const Project = a.entity("Project", {
  key: ["orgId", "projectId"],
  fields: {
    orgId: t.string().min(1).max(64),
    projectId: t.id().generated(),
    name: t.string().min(1).max(120),
    status: t.enum("active", "archived").default("active"),
  },
});

const activeProjects = a.counter("activeProjects", {
  of: Project,
  groupBy: ["orgId"],
  where: (p) => p.status.eq("active"),
});

a.route("POST /orgs/:orgId/projects")
  .body({ name: t.string().min(1).max(120) })
  .errors({ planLimit: err.forbidden("Project limit reached") })
  .handle(($, { params, body, errors }) => {
    $.ensure(activeProjects.get({ orgId: params.orgId }).lt(3), errors.planLimit);
    return Project.insert({ orgId: params.orgId, name: body.name });
  });

export default a;
```

The handler runs **once, at build time**, with symbolic values. Every operation is recorded into a
plan, and the plan is what the platform runs. The core computes every route's worst-case cost before
it's deployed:

```
createProject  reads ≤ 3  writes ≤ 3  ops ≤ 22  bytes read ≤ 1712
```

It rejects anything over the limits, such as an unbounded query or a loop that could write too much,
with a diagnostic an agent can act on. At runtime, the meter enforces the same limits.

See [`conformance/cases`](conformance/cases) for complete apps: a SaaS backend with plan limits and
bulk operations, and a ledger with transfers.

## How it fits together

```
app.ts ──@aleph/sdk──▶ manifest.json ──aleph-core──▶ validated plan ──▶ Engine::handle(tenant, request)
                                       (types, scopes,               (one optimistic transaction,
                                        worst-case cost)              metered, tenant-prefixed keys)
```

- **One core, everywhere.** `aleph-core` runs natively in the platform and as WASM in the toolchain
  (`@aleph/core`), so local results and production results come from the same code.
- **One source of truth for the format.** The Rust types generate the JSON Schema and the TypeScript
  types. `pnpm verify` fails if the generated files are stale.
- **One conformance suite.** The same golden files run through the SDK, the WASM build, and the
  native core.

The design rationale is in the `aleph-infra` repo (`spec/architecture.md`, `spec/feasibility.md`,
`spec/sdk-examples/guardrails.md`).

## Development

Requirements: Rust (pinned by `rust-toolchain.toml`, with the `wasm32-unknown-unknown` target),
`wasm-bindgen-cli` 0.2.130 (`cargo install wasm-bindgen-cli --version 0.2.130 --locked`), Node ≥ 24,
and pnpm (pinned by `packageManager`).

```bash
pnpm install
```

```bash
pnpm build        # builds the WASM core and all packages (Turborepo)
```

```bash
pnpm verify       # every check: rustfmt, clippy, cargo test, codegen check, Biome, build, typecheck, TS tests
```

There's no hosted CI for now: run `pnpm verify` before pushing.

| Task | Command |
|---|---|
| Format and auto-fix TS/JSON | `pnpm fix` |
| Format Rust | `cargo fmt --all` |
| Regenerate the schema and TS types after changing Rust manifest types | `pnpm codegen` |
| Rewrite golden conformance files after an intended change | `pnpm --filter @aleph/conformance update` |

Workspace packages resolve to their TypeScript sources through the `@aleph/source` export
condition, so tests and typechecking don't need a build. Published packages use `dist/`.

## Status

This is v0 (manifest format `0.1`). It covers entities, indexes, counters, routes, guards, branches,
bounded loops, `map`/`filter`, queries, and transactions. Not here yet: org roles and API keys in
auth, queues, cron, workflows, integrations, realtime channels, source spans in diagnostics, and the
`aleph` CLI. See the tickets in `aleph-infra/todo`.

## License

[Apache-2.0](LICENSE)
