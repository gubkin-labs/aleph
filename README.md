# Aleph Language

Aleph aims to be a declarative language and shared, multi-tenant runtime for building server-side applications. Apps describe the capabilities they need and the logic they are allowed to run; the platform supplies the managed runtime and common backend services.

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

HTTP APIs and webhooks, request-shape validation, routing, rate limits, and generated interfaces such as OpenAPI or MCP where appropriate. The gateway owns protocol entry and routing, then passes trusted tenant context to app logic; it does not make app-specific authorization decisions.

### 2. Identity and access

Authenticate users and services, resolve organization membership, and authorize access to capabilities and resources. Common B2B features could include roles and permissions, OIDC/SAML SSO, SCIM provisioning, service accounts, API keys, sessions, and audit events. The platform derives trusted tenant context from identity and membership; it must never trust a tenant ID supplied only in request data.

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

## Multi-file TOML example

The proposed source format uses an Aleph-defined import manifest; TOML itself does not define imports. The example demonstrates a shared schema referenced by both a calculation and a route.

- [Root manifest](examples/multi-file/aleph.toml)
- [Shared schema](examples/multi-file/schemas/common.toml)
- [Storage](examples/multi-file/storage/main.toml)
- [Calculation](examples/multi-file/logic/calculations.toml)
- [Route](examples/multi-file/routes/foo.toml)
