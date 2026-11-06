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
| [`packages/cli`](packages/cli) | `@aleph/cli`: the `aleph` command (`build`, `check`, `dev`, `openapi`, `mcp`) |
| [`conformance`](conformance) | Golden apps, manifests, bounds, and request scenarios that the SDK, WASM, and native core must all agree on |
| [`schema`](schema) | `manifest.schema.json`, for third-party SDKs and platforms |

## A taste

```ts
import { app, err, t } from "@aleph/sdk";

const a = app("projects", { roles: ["owner", "admin", "member"], apiKeyScopes: ["projects:read"] });

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
  .member("orgId", { roles: ["owner", "admin"] })
  .idempotent({ ttl: "24h" })
  .body({ name: t.string().min(1).max(120) })
  .errors({ planLimit: err.forbidden("Project limit reached") })
  .handle(($, { params, body, errors }) => {
    $.ensure(activeProjects.get({ orgId: params.orgId }).lt(3), errors.planLimit);
    return Project.insert({ orgId: params.orgId, name: body.name });
  });

a.route("GET /orgs/:orgId/projects")
  .member("orgId", { apiKeyScopes: ["projects:read"] })
  .query({ cursor: t.cursor() })
  .handle((_, { params, query }) =>
    Project.page({ prefix: [params.orgId], limit: 20, after: query.cursor }),
  );

export default a;
```

The handler runs **once, at build time**, with symbolic values. Every operation is recorded into a
plan, and the plan is what the platform runs. The core computes every route's worst-case cost before
it's deployed:

```
$ aleph check app.ts
✔ valid: worst-case cost per route (share of the route limit)

route                       reads  writes      ops    bytes read
get_orgs_orgId_projects   20 (8%)  0 (0%)   6 (0%)  24.0 KB (1%)
post_orgs_orgId_projects   3 (1%)  3 (6%)  12 (1%)  65.2 KB (3%)
```

The core rejects anything over the limits, such as an unbounded query or a loop that could write too much,
with a diagnostic an agent can act on. At runtime, the meter enforces the same limits.

The create route's bounds include its idempotency record: one read, one write, and up to one row
(64 KB) of stored response. `aleph dev app.ts` serves the app locally on the same core. Callers are faked with
`x-aleph-user`, `x-aleph-orgs: acme=owner`, `x-aleph-api-key`, and `idempotency-key` headers.

Asynchronous work is declared the same way, and it's bounded the same way:

```ts
const Process = a.queue("process", { payload: { orgId, docId: t.id() }, retries: 2, backoff: "1s" });
const ocr = a.integration("ocr", {
  provider: "http",
  operations: { extract: { mode: "call", input: { docId: t.id() }, output: { pages: t.int().min(0) } } },
});

a.onInsert("enqueueProcessing", Doc, (_, { payload }) =>
  Process.enqueue({ orgId: payload.after.orgId, docId: payload.after.docId }),
);
a.onQueue("processDoc", Process, (_, { payload }) => {
  const result = ocr.call("extract", { docId: payload.docId }); // journaled: retries reuse it
  Doc.update(payload, { status: "done", pages: result.pages });
  Feed.publish({ orgId: payload.orgId }, { docId: payload.docId, status: "done" }); // after commit
});
a.every("purgeFailed", "1d", ($, { payload }) => $.forEach(payload.batch, (doc) => doc.delete()), {
  scan: { entity: Doc, index: "byStatus", prefix: ["failed"], batch: 50 },
});
```

Effects (events, jobs, change triggers, channel messages, integration sends) are written to a
transactional outbox in the same commit as the writes, so a rolled-back or retried request never
releases them, and a committed one releases each exactly once. Handlers are checked against job
limits; failed jobs retry with exponential backoff and then go to a dead-letter handler.

See [`conformance/cases`](conformance/cases) for complete apps: a SaaS backend with plan limits and
bulk operations, a ledger with transfers, and a document pipeline with queues, retries, a
dead-letter handler, an OCR integration, a realtime channel, and a scanning cron.

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

Manifest format `0.2`. The core and SDK cover:
- **Data:** entities, indexes, invariants, append-only entities, counters and sums, serializable
  transactions, cursor pagination, `upsert`, `insertIfAbsent`.
- **Logic:** guards, branches, bounded loops, `map`/`filter`, string and number helpers.
- **Routes:** auth (signed-in users, org roles, API key scopes), idempotency keys, rate limits,
  descriptions, MCP tools.
- **Async:** events, entity-change triggers, queues with retries, backoff and dead letters, delayed
  jobs, cron (optionally scanning an entity in batches), realtime channel publishes.
- **Integrations:** journaled calls and after-commit sends, with mock providers for tests and `aleph dev`.
- **Tooling:** the `aleph` CLI (`build`, `check` with route and handler cost tables, `dev` with
  background processing, `openapi`, `mcp`), and source maps so diagnostics point at your code.

Platform-side capabilities aren't here yet: durable multi-step workflows, streaming LLM routes,
blob storage, search and vector indexes, channel subscriptions, and real providers behind
integrations. See the tickets in `aleph-infra/todo`.

## License

[Apache-2.0](LICENSE)
