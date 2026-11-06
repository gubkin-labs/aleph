import Link from "next/link";
import { Code } from "@/components/Code";

const SAMPLE = `import { app, err, t } from "@aleph/sdk";

const a = app("tasks", { roles: ["owner", "member"] });

const Task = a.entity("Task", {
  key: ["orgId", "taskId"],                 // one org's tasks are one ordered range
  fields: {
    orgId: t.string().max(64),
    taskId: t.id().generated(),
    title: t.string().min(1).max(200),
    done: t.bool().default(false),
  },
});

const open = a.counter("openTasks", { of: Task, groupBy: ["orgId"], where: (x) => x.done.not() });

a.route("GET /orgs/:orgId/tasks")
  .member("orgId")                          // org members only
  .query({ limit: t.int().min(1).max(50).default(20), cursor: t.cursor() })
  .handle(($, { params, query }) => ({
    open: open.get({ orgId: params.orgId }),  // O(1), maintained on every write
    page: Task.page({ prefix: [params.orgId], limit: query.limit, after: query.cursor }),
  }));`;

const CHECK = `$ aleph check app.ts
✔ valid: worst-case cost (share of the limit)

route                    reads  writes      ops    bytes read   memory
get_orgs_orgId_tasks  51 (20%)  0 (0%)  11 (1%)  81.0 KB (4%)   226.2 KB (3%)`;

const GUARANTEES = [
  [
    "Known worst-case cost",
    "Every route has a static bound on reads, writes, operations, bytes, effects, calls, and memory, checked before deploy and enforced while it runs.",
  ],
  [
    "Serializable by default",
    "Each request is one optimistic transaction with phantom protection. No lost updates, no half-written state.",
  ],
  [
    "Effects exactly once",
    "Events, jobs, emails, and messages go to a transactional outbox: a failed request never sends, a committed one sends once.",
  ],
  [
    "Isolation by construction",
    "Every key is built inside its tenant's keyspace. A plan can't name another tenant's data, so it can't touch it.",
  ],
  [
    "Types end to end",
    "One schema drives input validation, inferred response types, a generated client, OpenAPI, and MCP tools.",
  ],
  [
    "Safe to evolve",
    "Every schema change is classified as safe, needing a backfill, or breaking, before it ships.",
  ],
];

const STEPS = [
  [
    "Describe",
    "Write routes, entities, and logic with the TypeScript SDK. Handlers run once, at build time, and record a plan.",
  ],
  ["Compile", "The SDK emits a manifest: plain, versioned JSON. No author code ships, only data."],
  [
    "Check",
    "The core validates types and references and computes worst-case costs. Over-budget plans are rejected with diagnostics people and agents can act on.",
  ],
  [
    "Run",
    "The same core interprets the plan inside one metered transaction, natively or as WASM: locally, in tests, and in production.",
  ],
];

export default function Home() {
  return (
    <main className="home">
      <section className="hero">
        <p className="eyebrow">Open-source · Apache-2.0</p>
        <h1>Backends you can reason about before they run.</h1>
        <p className="lead">
          Aleph is a declarative language for backends. Describe routes, data, and logic in
          TypeScript. Aleph compiles them into a manifest whose every request has a known worst-case
          cost, runs it in one serializable transaction, and keeps tenants isolated by construction.
        </p>
        <div className="hero-actions">
          <Link href="/docs/quickstart/" className="button">
            Get started
          </Link>
          <Link href="/docs/" className="button button-secondary">
            Read the docs
          </Link>
        </div>
      </section>

      <section className="split">
        <div>
          <h2>Ordinary TypeScript in, a checked plan out</h2>
          <p>
            Routes look like the code you'd write anyway. The difference is what you get back: the
            worst case of every request, before anything is deployed.
          </p>
          <Code code={CHECK} lang="text" />
        </div>
        <Code code={SAMPLE} lang="ts" />
      </section>

      <section>
        <h2>What a declarative backend buys you</h2>
        <div className="grid">
          {GUARANTEES.map(([title, text]) => (
            <div key={title} className="tile">
              <h3>{title}</h3>
              <p>{text}</p>
            </div>
          ))}
        </div>
      </section>

      <section>
        <h2>How it works</h2>
        <ol className="steps">
          {STEPS.map(([title, text]) => (
            <li key={title}>
              <h3>{title}</h3>
              <p>{text}</p>
            </li>
          ))}
        </ol>
        <p className="muted">
          A manifest runs on any host that embeds the open-source core: your laptop with{" "}
          <code>aleph dev</code>, your tests, or a production runtime.
        </p>
      </section>

      <section>
        <h2>Everything a real backend needs</h2>
        <div className="grid grid-small">
          {[
            "Entities, indexes, and counters",
            "Auth, org roles, API keys",
            "Cursor pagination and range queries",
            "Exact decimals and time zones",
            "Queues, retries, and schedules",
            "Durable workflows with compensation",
            "Signed webhooks with dedupe",
            "Idempotency and rate limits",
            "Worked examples as tests",
            "Typed clients and OpenAPI",
            "MCP tools for agents",
            "Local dev with an inspector",
          ].map((item) => (
            <div key={item} className="chip">
              {item}
            </div>
          ))}
        </div>
      </section>

      <section className="cta">
        <h2>Build your first app in five minutes</h2>
        <Link href="/docs/quickstart/" className="button">
          Start the quickstart
        </Link>
      </section>
    </main>
  );
}
