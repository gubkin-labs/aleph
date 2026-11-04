# Aleph Language

Aleph is a declarative language for describing server-side applications. This repository owns the TypeScript SDK, manifest contract, documentation, and examples. The separate Aleph Language CLI repository contains the CLI that checks app projects and synthesizes manifests. The separate Aleph Platform repository contains the TypeScript backend, built on Hono, Drizzle, PostgreSQL, and Better Auth, that validates and runs manifests as a shared, multi-tenant service. Apps describe the capabilities they need and the bounded logic they are allowed to run; the platform supplies the managed runtime and common backend services.

## Goals

- Build a broad range of backend applications from a small set of composable, declarative capabilities.
- Reduce per-application cost by safely sharing compute and managed services across tenants.
- Make tenant identity, data boundaries, and resource budgets explicit and enforceable.
- Provide useful defaults—such as authentication, API validation, logging, and deployment checks—without requiring each app to assemble them from scratch.
- Keep the platform supportable: offer a small number of well-defined capabilities with maintained provider adapters, rather than promising support for every backend or service.

Declarative configuration can simplify deployment, but it does not remove the need for validation, tests, or operational visibility. The compiler should reject invalid references, types, capabilities, and resource limits before deployment.

## Design principles

- **Explicit capabilities:** an app can access only the storage, messaging, identity, and provider capabilities declared for it.
- **Bounded logic:** tenant logic uses a restricted, inspectable plan; arbitrary tenant code does not run in the shared host.
- **Distinct execution contracts:** short requests, durable work, and connected real-time sessions have different runtimes and limits.
- **Tenant-aware by default:** every request and background operation carries a trusted tenant context; quotas and access checks apply across all capabilities.
- **Portable core, curated adapters:** the language describes stable contracts; the platform documents which implementations are supported at each maturity level.

## Proposed platform pillars

### 1. API and gateway

HTTP APIs and webhooks, request-shape validation, routing, rate limits, and generated interfaces such as OpenAPI. The gateway owns protocol entry and routing, then passes trusted tenant context to app logic; it does not make app-specific authorization decisions.

MCP is another protocol binding. An MCP server can declare server instructions and tools with names and descriptions. Each tool references a reusable operation and inherits its input/output schemas and authorization. The same operation can back an HTTP route and an MCP tool. Instructions guide clients; they are not a security control. Prompts and resources can be considered as separate capabilities.

### 2. Identity and access

Compose named auth profiles from platform-supported identity providers and feature plugins. Providers can cover email/password, passkeys, OIDC, and SAML; plugins can add organizations, teams, roles and permissions, invitations, MFA, SCIM, API keys, and audit events. Profiles bind to gateway routes and MCP servers, while reusable operations enforce authorization policies. Built-in user, session, organization, membership, and team entities are referenceable and safely extensible. See the [auth syntax draft](examples/auth-syntax.md). The platform derives trusted tenant context from identity and membership; it must never trust a tenant ID supplied only in request data.

### 3. Data and storage

Start with a few useful storage contracts:

- Relational storage for transactional application data.
- Key-value storage for simple lookups and ephemeral coordination.
- Object or file storage for large blobs.
- Optional cache and search/vector-index capabilities where a use case needs them.

Storage providers should preserve tenant scoping and declare their consistency, limits, and supported operations. Data owns generic index storage and search primitives; AI capabilities can generate embeddings and orchestrate retrieval. Vector indexes are derived data, not the source of truth.

### 4. Logic and execution

Expose execution modes by their guarantees, rather than treating every task as a generic function:

- **Functions:** short-lived request or event handlers with bounded execution.
- **Workflows:** durable, resumable processes with persisted progress, business-step retries, waits, and timers. They own orchestration state, not queue delivery mechanics.
- **Live:** event-driven logic for WebSocket connections, rooms, subscriptions, and presence.

Model operations as ordered, typed pipelines. Candidate steps include `authorize`, `derive`, `query`, `mutate`, `transform`, `call`, `publish`, `start_workflow`, and `return`. Each step is a built-in, inspectable primitive—not an arbitrary callback. A step binds results with `as`; later steps reference typed values such as `$cart.total`, alongside `$input` and trusted `$auth` context. Queries use a typed query structure, apply tenant scoping from table metadata, and have explicit result bounds.

For control flow, `match` should select among finite, typed cases; its worst-case cost is the most expensive branch. Collection work should use a dedicated `for_each` with a literal maximum item count and concurrency bound. This gives the validator a cost model: sequential steps sum, `match` takes the maximum branch cost, `for_each` multiplies body cost by its item bound, and fixed `parallel` calls sum total CPU cost while limiting peak concurrency. No general `while` loops or recursion. Request-time async calls need deadlines, response-size, and concurrency limits; long waits belong in workflows, where progress is persisted and compute can be released. Static estimates should include retry bounds and be backed by runtime fuel and per-tenant budgets.

Cron is a trigger for a function or workflow, not a separate execution model. The gateway owns WebSocket protocol entry; Live owns connection/session behavior.

### 5. Events and messaging

- **Queues** own message delivery: durable handoff, acknowledgements, delivery retries, and dead-letter handling.
- **Pub/sub** owns event fan-out to multiple subscribers.

These are different delivery contracts. Either can trigger a function or workflow. Workflows own business-step state and retries; app authors should not need to operate the platform's internal queues.

### 6. AI capabilities

Provide managed interfaces for model providers, chat/conversation flows, streaming responses, and embedding generation. AI owns model calls and retrieval orchestration; the data pillar owns vector-index storage/search. AI tools should call explicitly declared app capabilities. Conversation history belongs in storage, and long-running AI tasks can use workflows. Apply tenant-specific credentials, usage budgets, and data-handling policies.

## Shared platform responsibilities

These are cross-cutting guarantees, not separate app-level pillars:

- Tenant isolation for data, secrets, execution, and events; trusted tenant context follows work across requests, messages, and workflows.
- Per-tenant and per-execution limits for CPU, memory, concurrency, connections, payloads, storage, and provider usage.
- Validation at the right boundary: the compiler checks configuration and references before deployment, the gateway validates incoming payloads, and runtimes enforce resource budgets.
- Retries, idempotency, timeouts, cancellation, and failure reporting appropriate to each execution mode.
- Logs, metrics, traces, audit records, and usage metering with tenant context.
- Versioned deployments and a clear supported-provider matrix.

## Suggested delivery order

1. **Foundation:** versioned manifest/IR, validation, tenant context and limits, HTTP gateway, identity, relational and basic key-value storage, bounded request functions, and observability.
2. **Durable async:** queues, pub/sub, workflows, and cron triggers.
3. **Stateful and provider-heavy capabilities:** Live/WebSockets, then managed AI interfaces and vector retrieval.

Keep the core contracts stable and add a small number of platform-maintained adapters at each stage. Avoid arbitrary tenant plugins in the shared runtime; they make isolation, support, and cost guarantees much harder.

## TypeScript SDK, CLI, and manifest boundary

The TypeScript SDK is an authoring frontend, not the tenant runtime. Schema and builder calls create typed, serializable plan nodes. The separate `aleph-language-cli` package type-checks the project and synthesizes those nodes into a versioned manifest; it does not translate arbitrary TypeScript into runtime code. Custom JavaScript callbacks are not serializable plan nodes—use Aleph's expression and operation builders instead.

The implemented workflow currently includes:

1. `aleph check <entry.ts>` type-checks the TypeScript project, loads its exported manifest, and validates the Aleph definitions.
2. `aleph build <entry.ts> --out aleph.manifest.json` performs the same checks and writes deterministic, versioned JSON.
3. `aleph-platform deploy <manifest.json>` (from the separate platform CLI) uploads the artifact; the platform independently validates it before activating an immutable app revision.

Loading an entry module evaluates project code at build time. Run builds on the developer's machine or trusted CI; any hosted builder must sandbox this step. The platform never evaluates the app's TypeScript. App changes publish a manifest revision rather than an app-specific binary or container; the shared platform runtime is still deployed and operated separately. TypeScript's compile-time checks do not replace platform validation.

The SDK package is built with `tsdown`, including TypeScript declarations. The CLI uses Commander for subcommands and Ink for terminal UI, falling back to plain output in non-interactive environments. The current executable subset covers structured schemas, operations with bounded expressions, and HTTP route bindings. The broader `examples/multi-file` sketches document capabilities still to be implemented.

### Local SDK and CLI development

Use Node.js 24 and pnpm 10. This SDK repository has no workspace; the CLI lives in its own repository.

```bash
pnpm install
pnpm check
pnpm build

cd ../aleph-language-cli
pnpm install
pnpm build
cd ../aleph-language
node ../aleph-language-cli/dist/index.mjs check examples/quickstart/app.ts
node ../aleph-language-cli/dist/index.mjs build examples/quickstart/app.ts --out /tmp/quickstart.manifest.json
```

`examples/quickstart/app.ts` is the executable SDK-to-manifest smoke test.

### Versioning and independent releases

SDK and CLI package versions are distinct from the manifest format version. Manifests identify their format version and required features; the platform explicitly supports selected format versions and features. This lets the language tooling release independently while the platform adopts compatibility at its own pace. The platform consumes the manifest contract and artifacts—not the TypeScript SDK or CLI as runtime dependencies—and should reject unsupported versions or features before activation.

The language tooling and platform are separate repositories and can have separate release and licensing decisions. Keeping the manifest contract versioned and documented allows the authoring tools to be open source even if the platform's open-source status is decided independently.

## Backend application validation portfolio

Build these as small, end-to-end slices in roughly this order. Each app should test a real combination of SDK and runtime behavior; the portfolio is a way to expose missing patterns, not a promise that twenty examples alone prove 90% coverage.

1. **Personal tasks and notes:** CRUD, validation, authentication, filtering, and cursor pagination.
2. **Multi-tenant project tracker:** organizations, teams, roles, tenant-scoped queries, comments, and audit history.
3. **Headless CMS:** drafts, publishing, revisions, localization, and scheduled publication.
4. **Appointment booking:** availability, time zones, expiring holds, and concurrency-safe reservations.
5. **Online shop checkout:** carts, inventory, idempotency, payment calls, and payment webhooks.
6. **Subscription SaaS:** trials, plan changes, entitlements, renewals, and recurring workflows.
7. **File and media library:** object storage, signed access, metadata, and asynchronous processing.
8. **Notification service:** preferences, provider calls, fan-out, retries, deduplication, and digests.
9. **Customer support desk:** ticket transitions, assignment, SLA timers, attachments, and audit events.
10. **Webhook delivery service:** endpoint registration, signatures, retries, dead letters, and replay.
11. **Third-party data sync:** OAuth secret references, pagination cursors, rate limits, schedules, and resumable jobs.
12. **Realtime collaboration chat:** rooms, authorization, presence, subscriptions, and message history.
13. **Developer API product:** API keys, service identities, quotas, rate limits, idempotency, and generated API documentation.
14. **Expense approval system:** multi-step approvals, delegation, reminders, durable waits, and audit history.
15. **IoT telemetry and alerts:** bounded ingestion, tenant budgets, retention, aggregation, and event-triggered workflows.
16. **Usage metering and analytics:** event capture, rollups, late/duplicate events, quota checks, and exports.
17. **Multi-vendor marketplace:** buyer/seller boundaries, order state, refunds, payouts, and financial audit records.
18. **Community forum:** feeds, search, moderation, visibility rules, and rate limits.
19. **Learning management system:** courses, enrollments, progress, prerequisites, cohorts, and scheduled reminders.
20. **AI knowledge base:** document ingestion, retrieval, streaming responses, bounded tool calls, and per-tenant model budgets.

## TypeScript SDK examples and sketches

- [Executable manifest quickstart](examples/quickstart/app.ts)
- [Composed application design sketch](examples/multi-file/app.ts)
- [Reusable schemas](examples/multi-file/schemas/common.ts)
- [Storage definitions](examples/multi-file/storage/main.ts)
- [Composed auth profile](examples/multi-file/auth/workforce.ts)
- [Bounded calculation](examples/multi-file/logic/calculations.ts)
- [Shared order operation](examples/multi-file/logic/orders.ts)
- [HTTP calculation route](examples/multi-file/routes/foo.ts)
- [HTTP order route](examples/multi-file/routes/orders.ts)
- [MCP server and shared tool](examples/multi-file/mcp/server.ts)
- [Auth syntax and model extensions](examples/auth-syntax.md)
- [Logic and cost-model stress tests](examples/syntax-stress-tests.md)
