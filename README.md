# Aleph

Aleph is a declarative language for backends. You describe routes, data, and logic in TypeScript.
The SDK compiles that into a versioned **manifest**, and a platform runs the manifest on shared
infrastructure. Every operation is bounded, so thousands of apps can share the same processes and
databases safely.

## Install

In a project on Node.js 24 or later:

```bash
npm install @aleph-lang/sdk
npm install --save-dev @aleph-lang/cli
npm pkg set type=module   # Aleph apps are ES modules; new npm projects default to CommonJS
```

Then follow the [quickstart](https://aleph-lang.vercel.app/docs/quickstart/).

## What's here

This repository is the open-source half of Aleph:

| Path | What it is |
|---|---|
| [`crates/aleph-core`](crates/aleph-core) | The trusted core (Rust): manifest types, validator, static usage analyzer, bounded plan interpreter, runtime usage meter, tenant-isolated key encoding, and an in-memory transactional store |
| [`crates/aleph-core-wasm`](crates/aleph-core-wasm) | `aleph-core` compiled to WASM |
| [`crates/xtask`](crates/xtask) | Code generation: JSON Schema and TypeScript types from the Rust types |
| [`packages/manifest`](packages/manifest) | `@aleph-lang/manifest`: generated TypeScript types for the manifest format |
| [`packages/core-wasm`](packages/core-wasm) | `@aleph-lang/core`: validation, usage analysis, and a local dev engine (the Rust core, via WASM) |
| [`packages/sdk`](packages/sdk) | `@aleph-lang/sdk`: the TypeScript builder that produces manifests |
| [`packages/cli`](packages/cli) | `@aleph-lang/cli`: the `aleph` command (`build`, `check`, `test`, `client`, `dev`, `openapi`, `mcp`) |
| [`examples`](examples) | Example apps with a minimal React UI and an inspector panel (`pnpm --filter @aleph-examples/shop dev`) |
| [`website`](website) | The documentation site (Next.js, static export): `pnpm --filter @aleph-lang/website dev` |
| [`conformance`](conformance) | Golden apps, manifests, bounds, and request scenarios that the SDK, WASM, and native core must all agree on |
| [`schema`](schema) | `manifest.schema.json`, for third-party SDKs and platforms |

## A taste

```ts
import { app, err, t } from "@aleph-lang/sdk";

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
plan, and the plan is what the platform runs. The core computes the most every route can use before
it's deployed:

```
$ aleph check app.ts
✔ valid. The most each route and handler can use in one run:

route                       reads   writes         ops   bytes read   effects   calls         memory
limit                      250       50      2,000          2 MB         50       3         8 MB
get_orgs_orgId_projects     20 8%     0  –       6 <1%   24.0 KB 1%       0 –     0 –   112.1 KB  1%
post_orgs_orgId_projects     3 1%     3 6%      12 <1%   65.2 KB 3%       0 –     0 –     2.4 KB <1%
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

Objects, exact decimals, time zones, response types, worked examples, and webhooks:

```ts
const Order = a.entity("Order", {
  key: ["customerId", "orderId"],
  fields: {
    customerId: t.string().max(64),
    orderId: t.id().generated(),
    items: t.array(t.object({ sku: t.string().max(32), qty: t.int().min(1), unitPrice: t.decimal(2) }), 10),
    total: t.decimal(2),
    tz: t.timeZone().default("UTC"),
    placedAt: t.timestamp().defaultNow(),
  },
});

a.route("POST /orders")
  .body({ items: t.array(t.object({ sku: t.string().max(32), qty: t.int().min(1).max(99) }), 10) })
  .status(201)
  .errors({ unknownProducts: err.unprocessable("Some products don't exist", { missing: t.int() }) })
  .example({
    name: "totals exactly",
    as: { userId: "c1" },
    given: { Product: [{ sku: "BK-1", name: "Rust", price: "19.99", category: "books" }] },
    request: { body: { items: [{ sku: "BK-1", qty: 3 }] } },
    expect: { status: 201, body: { total: "59.97" } },
  })
  .handle(($, { body, errors }) => {
    const lines = body.items.map((item) => {
      const product = Product.get({ sku: item.sku });
      return { sku: item.sku, qty: item.qty, found: product.exists(), price: product.price.ifNull($.decimal("0")) };
    });
    $.ensure(lines.every((l) => l.found), errors.unknownProducts, { missing: lines.count((l) => l.found.not()) });
    const total = lines.sum((l) => l.price.times(l.qty)).round(2);
    const order = Order.insert({ customerId: $.caller, items: lines.map(/* ... */), total });
    $.header("location", $.fmt`/orders/${order.orderId}`);
    return { orderId: order.orderId, total };
  });

a.webhook("POST /webhooks/payments", { verify: verify.stripe("STRIPE_WEBHOOK"), dedupe: { bodyField: "id" } })
  .body({ id: t.string().max(64), orderId: t.id() })
  .handle(/* ... */);
```

`aleph test` runs every `.example(...)`, and `aleph client app.ts -o client.ts` writes a typed
fetch client from the inferred (or declared, `.returns(...)`) response types.

See [`conformance/cases`](conformance/cases) for complete apps:
- a SaaS backend with plan limits and bulk operations
- a ledger with transfers
- a document pipeline with queues, retries, a dead-letter handler, an OCR integration, a realtime
  channel, and a scanning cron
- a shop (`commerce`) that exercises nearly every v0.3 feature
- order fulfillment (`fulfillment`) as a durable workflow with compensation

## How it fits together

```
app.ts ──@aleph-lang/sdk──▶ manifest.json ──aleph-core──▶ validated plan ──▶ Engine::handle(tenant, request)
                                       (types, scopes,               (one optimistic transaction,
                                        worst-case usage)             metered, tenant-prefixed keys)
```

- **One core, everywhere.** `aleph-core` runs natively in the platform and as WASM in the toolchain
  (`@aleph-lang/core`), so local results and production results come from the same code.
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

### Fuzzing

`fuzz/` holds coverage-guided fuzz targets for `aleph-core`, run with
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz). It's its own Cargo workspace on nightly,
so `pnpm verify` doesn't build it. One-time setup:

```bash
rustup toolchain install nightly --profile minimal && cargo install cargo-fuzz --locked
```

```bash
fuzz/seed.sh      # seeds parse_bytes with the golden and invalid conformance manifests
```

Then run a target from `fuzz/` (`-s none`: the core has no `unsafe`, so AddressSanitizer only
slows it down; `-fork=N` runs N workers and keeps going after a failure):

```bash
cd fuzz && cargo +nightly fuzz run -s none -O execute -- -max_total_time=1800 -timeout=10 -fork=4 -ignore_crashes=1
```

| Target          | Input                                    | Checks                                                                                                     |
| --------------- | ---------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `parse_bytes`   | arbitrary bytes (add `-dict=manifest.dict`) | `parse_and_validate` returns a manifest or diagnostics: no panic, stack overflow, or timeout              |
| `parse_mutated` | golden manifests, structure-aware mutations | the same, plus every accepted route has bounds within the limits                                          |
| `execute`       | a mutated manifest that validates, part of its scenario, then generated requests, `process` calls, and clock advances | usage ≤ static bounds, no `budget_exceeded`, only the executing tenant's keys change, a failed request leaves the store unchanged, no panic |

`ALEPH_FUZZ_STRICT=1` also fails `execute` on `runtime_error` responses (a checker gap rather
than a broken promise). Failures land in `fuzz/artifacts/<target>/`; reproduce one with
`cargo +nightly fuzz run -s none -O <target> <file>`, shrink it with `cargo +nightly fuzz tmin`,
and add it as a regression test in `crates/aleph-core/tests/` before fixing the core.

### Releasing

Releases are manual, from your machine with your own npm login:

```bash
pnpm release 0.1.0
```

`scripts/release.sh` checks you're on a clean, pushed `main` and logged in to npm and GitHub, runs
`pnpm verify`, packs `@aleph-lang/manifest`, `@aleph-lang/core`, `@aleph-lang/sdk`, and
`@aleph-lang/cli` (`scripts/release-pack.mjs`), publishes them to npm (npm may ask you to confirm
in the browser), then tags the commit `v<version>` and creates the GitHub release. The packages
depend on each other at the exact same version. A prerelease like `0.2.0-beta.1` goes to the npm
tag `next`, so `npm install` keeps getting the latest stable release. If it fails partway, run it
again with the same version: packages already on npm are skipped.

The **Release** workflow in GitHub Actions does the same from CI, with npm provenance. It needs an
`NPM_TOKEN` repository secret that can publish to `@aleph-lang`.

| Task | Command |
|---|---|
| Format and auto-fix TS/JSON | `pnpm fix` |
| Format Rust | `cargo fmt --all` |
| Regenerate the schema and TS types after changing Rust manifest types | `pnpm codegen` |
| Rewrite golden conformance files after an intended change | `pnpm --filter @aleph-lang/conformance update` |

Workspace packages resolve to their TypeScript sources through the `@aleph/source` export
condition, so tests and typechecking don't need a build. Published packages use `dist/`.

## Status

Manifest format `0.2` (language v0.3 features are additive). The core and SDK cover:
- **Data:** entities with string, int, exact decimal, bool, enum, id, timestamp, time zone, list,
  and nested object fields. Indexes can be unique, partial (`where`), or computed (`lower(email)`).
  Queries take a prefix, a multi-value prefix (`anyOf`, like SQL's `IN`), or a range on the next
  field. Also: invariants, append-only and versioned entities (with `ifVersion` updates), row TTL,
  and parent relations with `childrenOf` and cascading deletes. Counters cover counts, sums,
  min/max/latest, and distinct counts, can be grouped by computed parts (per day), and can be
  sharded for hot keys. Plus serializable transactions, cursor pagination, `upsert`, and
  `insertIfAbsent`.
- **Logic:** guards, branches, early `return`, bounded loops, `map`/`filter`, and list helpers
  (`sum`, `min`, `max`, `some`, `every`, `count`, `sortBy`, `slice`, `distinct`, `concat`,
  `includes`). String helpers: `split`, `replace`, `slice`, `pad`, linear-time `matches`. Exact
  decimal math with rounding modes, and integer `mod`, `pow`, `divFloor`, `divCeil`. Calendar math
  in IANA time zones (`startOf`, `add`, `part`, `formatTime`), and `null` that propagates like SQL.
- **Routes:** auth (signed-in users, org roles, API key scopes), caller profiles and a user
  directory, idempotency keys, rate limits, status codes, headers, cookies, redirects, errors with
  typed details, declared or inferred response types, CORS, deprecation, MCP tools, and inbound
  webhooks (HMAC, Stripe, GitHub signatures, with event dedupe).
- **Async:** events, entity-change triggers, queues with retries, backoff and dead letters, delayed
  and keyed jobs (replace or cancel by key), interval and calendar schedules (`"mon 09:00"` in a
  time zone), optionally scanning an entity in batches, and realtime channel publishes.
- **Workflows:** durable multi-step processes. Each step is a bounded transaction with typed state
  carried between steps, signals with timeouts (early signals are kept), delays, retries,
  compensation steps, cancellation, and `maxSteps` so every instance finishes.
- **Integrations:** journaled calls and after-commit sends, with mock providers for tests and `aleph dev`.
- **Guarantees:** worst-case reads, writes, ops, bytes, effects, calls, and memory for every route
  and handler, enforced by a runtime meter. A differential test over 100k+ generated plans checks
  that actual usage never exceeds the bounds and that runs never leave their tenant.
- **Tooling:** the `aleph` CLI:
  - `build`
  - `check`, with route and handler usage tables, and `--against` to classify schema changes and
    print a migration plan
  - `test`, which runs route examples
  - `client`, which writes a typed fetch client
  - `dev`, with background processing, `--watch`, `--seed`, persistent `--data`, `--secret`, and logs
  - `openapi`, with response schemas, and `mcp`

  Source maps make diagnostics point at your code.

Platform-side capabilities aren't here yet: streaming LLM routes,
blob storage, search and vector indexes, channel subscriptions, and real providers behind
integrations. See the tickets in `aleph-infra/todo/platform`.

## License

[Apache-2.0](LICENSE)
