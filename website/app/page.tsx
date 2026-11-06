import Link from "next/link";
import type { ReactNode } from "react";
import { Code } from "@/components/Code";
import {
  AgentScene,
  CostScene,
  DescribeScene,
  GeneratedScene,
  PredictableScene,
  SharedScene,
} from "@/components/Scenes";

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

route                    reads  writes      ops    bytes read  effects   calls         memory
get_orgs_orgId_tasks  51 (20%)  0 (0%)  11 (1%)  81.0 KB (4%)   0 (0%)  0 (0%)  226.2 KB (3%)`;

function Chapter({
  title,
  scene,
  flip,
  children,
}: {
  title: string;
  scene: ReactNode;
  flip?: boolean;
  children: ReactNode;
}) {
  return (
    <section className={flip ? "chapter chapter-flip" : "chapter"}>
      <div className="chapter-text">
        <h2>{title}</h2>
        {children}
      </div>
      <div className="chapter-scene">{scene}</div>
    </section>
  );
}

export default function Home() {
  return (
    <main className="home">
      <section className="hero">
        <h1>Describe a backend. Know its worst case before it runs.</h1>
        <p className="lead">
          Aleph is an open-source, declarative language for backends. You describe routes, data, and
          logic; Aleph turns the description into a plan and computes the most any request can ever
          cost. That one fact is what lets many apps share the same servers and database safely, and
          sharing is what makes them cheap to run.
        </p>
        <div className="hero-actions">
          <Link href="/docs/quickstart/" className="button">
            Get started
          </Link>
          <Link href="/docs/why-declarative/" className="button button-secondary">
            Why declarative
          </Link>
        </div>
      </section>

      <Chapter title="You describe what happens, not how" scene={<DescribeScene />}>
        <p>
          A route in Aleph is a description: which rows it reads, what it writes, what it checks.
          The SDK runs your handler once, at build time, and records it as data. Nothing you wrote
          runs in production; a small, audited interpreter does.
        </p>
        <p>
          Because the program is data, Aleph can read it the way a reviewer would, and work out what
          it does in the worst case without running it.
        </p>
      </Chapter>

      <Chapter title="Every request has a ceiling" scene={<PredictableScene />} flip>
        <p>
          Every route gets a bound on reads, writes, operations, bytes, memory, effects, and
          outbound calls. A route that could exceed its budget is rejected by{" "}
          <code>aleph check</code>, with a message saying which query drives the cost.
        </p>
        <p>
          While it runs, the same bound is metered. No request discovers in production that it loops
          a million times.
        </p>
      </Chapter>

      <Chapter title="So apps can share infrastructure" scene={<SharedScene />}>
        <p>
          Putting unrelated apps on one server is usually risky: one bad query or runaway loop slows
          everyone down. With Aleph that can't happen, because no request can grow past a bound
          known in advance, and every key lives inside its own tenant's keyspace.
        </p>
        <p>
          So a runtime can pack thousands of small apps into the same processes and the same
          database, the way a hosting provider packs static sites.{" "}
          <Link href="/docs/shared-infrastructure/">How sharing works</Link>
        </p>
      </Chapter>

      <Chapter title="Which is why it costs less" scene={<CostScene />} flip>
        <p>
          A typical small backend pays for a server, a database, and a cache that sit idle most of
          the day. Shared, the idle capacity is pooled; you pay for requests that happen.
        </p>
        <p>
          And because the worst case of every route is known, so is the most a route can cost you,
          before you deploy it.
        </p>
      </Chapter>

      <Chapter title="One description, everything derived from it" scene={<GeneratedScene />}>
        <p>
          The same declaration validates input, infers response types, and produces OpenAPI, a typed
          client, and MCP tools. Worked examples you write next to a route run as tests. Every
          schema change is classified as safe, needing a backfill, or breaking.
        </p>
        <p>
          There is nothing to wire up: no ORM, no queue client, no migration scripts, no separate
          API spec to keep in sync.
        </p>
      </Chapter>

      <Chapter title="Small enough for an agent to get right" scene={<AgentScene />} flip>
        <p>
          The vocabulary is closed and typed, so a coding agent can't reach for a library that isn't
          there. When it writes something too expensive or unsafe, the checker answers with a
          precise diagnostic, and the agent can fix its own work before anyone reviews it.
        </p>
      </Chapter>

      <section className="sample">
        <h2>What it looks like</h2>
        <p className="muted">
          Ordinary TypeScript in, a checked plan out. This route lists an org's tasks with an open
          count, and <code>aleph check</code> reports its worst case.
        </p>
        <Code code={SAMPLE} lang="ts" />
        <Code code={CHECK} lang="text" />
      </section>

      <section className="cta">
        <h2>Build a first app in five minutes</h2>
        <p className="muted">
          Install the CLI, write one file, run it locally with <code>aleph dev</code>.
        </p>
        <Link href="/docs/quickstart/" className="button">
          Start the quickstart
        </Link>
      </section>
    </main>
  );
}
