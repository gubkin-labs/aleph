import Link from "next/link";
import type { ReactNode } from "react";
import { Install } from "@/components/Install";
import { type Line, Panel, Replay } from "@/components/Replay";
import { SharedScene } from "@/components/Scenes";

// Every session below is real output from the CLI, run on the quickstart's notes app.

const CHECK: Line[] = [
  { k: "cmd", t: "$ aleph check app.ts" },
  { k: "ok", t: "✔ valid: worst-case cost (share of the limit)" },
  { t: "" },
  {
    k: "dim",
    t: "route                    reads  writes      ops    bytes read  effects   calls         memory",
  },
  {
    t: "get_notes              20 (8%)  0 (0%)   6 (0%)  67.6 KB (3%)   0 (0%)  0 (0%)  199.3 KB (2%)",
  },
  {
    t: "post_notes              1 (0%)  1 (2%)   7 (0%)   3.4 KB (0%)   0 (0%)  0 (0%)    6.8 KB (0%)",
  },
  {
    t: "post_notes_noteId_pin   2 (1%)  1 (2%)  16 (1%)   6.8 KB (0%)   0 (0%)  0 (0%)   10.1 KB (0%)",
  },
];

const GENERATE: Line[] = [
  { k: "cmd", t: "$ aleph build app.ts -o manifest.json" },
  { t: "wrote manifest.json" },
  { k: "cmd", t: "$ aleph client app.ts -o client.ts" },
  { t: "wrote client.ts" },
  { k: "cmd", t: "$ aleph openapi app.ts > openapi.json" },
  { k: "cmd", t: "$ aleph mcp app.ts" },
  { t: '{ "tools": [ { "name": "list_notes", "description": "GET /notes", ... } ] }' },
];

const EVOLVE: Line[] = [
  { k: "cmd", t: "$ aleph check app.ts --against deployed.json" },
  {
    k: "err",
    t: "✖ entities.Note.fields.text: string length 1-500 narrowed to 1-200; existing values may not fit",
  },
  { k: "ok", t: "✔ entities.Note.fields.color: new field color" },
  { t: "" },
  {
    k: "err",
    t: "✖ not deployable: breaking changes need a deprecation in the deployed manifest first",
  },
];

const TEST: Line[] = [
  { k: "cmd", t: "$ aleph test app.ts" },
  { k: "ok", t: "✔ post_notes: creates a note" },
  { t: "1 passed, 0 failed" },
  { k: "cmd", t: "$ aleph dev app.ts --watch" },
  { t: "aleph dev: serving app.ts on http://127.0.0.1:3000" },
  { k: "dim", t: "POST /notes → 201 post_notes (reads 1, writes 1, ops 7) 1.43ms" },
  { k: "dim", t: "GET /notes → 200 get_notes (reads 0, writes 0, ops 6) 7.8ms" },
];

const AGENT: Line[] = [
  { k: "agent", t: "agent ▸ Note.page({ prefix: [$.caller], limit: 5000, after: query.cursor })" },
  { k: "cmd", t: "$ aleph check app.ts" },
  { k: "err", t: "✖ over_budget at app.ts:28:4 (routes[1].plan)" },
  { t: '  route "get_notes" can use up to 5000 reads in the worst case, over the limit of 250.' },
  { t: "  Lower the bounds that drive it (query limits, list sizes) or split the work." },
  { k: "dim", t: "  …" },
  { t: "3 problems" },
  { k: "agent", t: "agent ▸ Note.page({ prefix: [$.caller], limit: 20, after: query.cursor })" },
  { k: "cmd", t: "$ aleph check app.ts && aleph test app.ts" },
  { k: "ok", t: "✔ valid: worst-case cost (share of the limit)" },
  { t: "1 passed, 0 failed" },
];

function Section({
  panel,
  title,
  children,
}: {
  panel: ReactNode;
  title: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="row">
      <div className="row-panel">{panel}</div>
      <div className="row-text">
        <h2>{title}</h2>
        {children}
      </div>
    </section>
  );
}

export default function Home() {
  return (
    <main className="home">
      <section className="hero">
        <div className="hero-mark" aria-hidden>
          ℵ
        </div>
        <h1>
          A backend shouldn’t cost <em>$100 a month</em> before its first user
        </h1>
        <p className="lead">
          Aleph is a declarative language for backends. Every request’s cost is known before it
          runs, so thousands of apps can safely share the same servers and database, and each one
          costs a fraction of its own stack.
        </p>
        <Install />
      </section>

      <Section
        title="Know the cost before you deploy"
        panel={
          <Replay
            title="aleph · worst case of every route"
            lines={CHECK}
            caption="The numbers are upper bounds, computed from the code, not measured."
          />
        }
      >
        <p>
          You describe routes, data, and logic in ordinary TypeScript. Aleph records that
          description instead of running it, and works out the most each route can ever read, write,
          compute, and hold in memory.
        </p>
        <p>
          A route that could go over its limit is rejected before it ships, with a message saying
          which query to change. The same limits are metered while it runs.
        </p>
      </Section>

      <Section
        title="Safe to share, so cheap to run"
        panel={
          <Panel
            title="aleph · many apps, one runtime"
            caption="Nothing an app does can grow past its known bound, so none can slow down the rest."
          >
            <SharedScene />
          </Panel>
        }
      >
        <p>
          Most small backends pay for a server, a database, and a cache that sit idle most of the
          day. They need their own because one runaway query in a shared setup slows down everyone
          else.
        </p>
        <p>
          In Aleph that can’t happen. Every request has a ceiling, and every key lives in its own
          app’s keyspace. So a runtime can pack thousands of apps into the same processes and the
          same database, the way static sites share a CDN. Capacity is pooled, so an app with no
          traffic costs next to nothing.{" "}
          <Link href="/docs/shared-infrastructure/">How sharing works</Link>
        </p>
      </Section>

      <Section
        title="Nothing to operate"
        panel={
          <Replay
            title="aleph · one description, everything derived"
            lines={GENERATE}
            caption="The manifest is plain JSON. No author code ships, only data."
          />
        }
      >
        <p>
          The app compiles into a manifest: a versioned description that any Aleph runtime can run.
          There are no servers to size, no Dockerfiles, no deploy pipeline, and no ORM, queue
          client, or auth library to keep upgraded.
        </p>
        <p>
          The same description gives you an OpenAPI spec, a typed client, and MCP tools for agents.
          And since only data ships, the engine underneath can get faster and safer without you
          changing a line.
        </p>
      </Section>

      <Section
        title="Change it without fear"
        panel={
          <Replay
            title="aleph · compare with what's deployed"
            lines={EVOLVE}
            caption="Every schema change is safe, needs a backfill, or is breaking. You find out first."
          />
        }
      >
        <p>
          Each request runs as one serializable transaction, and events, emails, and jobs go out
          exactly once, only after it commits. There are no half-written states to clean up.
        </p>
        <p>
          When you change the schema, Aleph compares it with what’s deployed and blocks changes that
          would break existing data until you deprecate them properly.
        </p>
      </Section>

      <Section
        title="Examples are the tests"
        panel={
          <Replay
            title="aleph · test and serve locally"
            lines={TEST}
            caption="Each example runs on a fresh store and also checks the route stayed within its bound."
          />
        }
      >
        <p>
          Write examples next to a route: given this data and this request, expect this response.
          They document the route and run as its tests.
        </p>
        <p>
          <code>aleph dev</code> serves the app locally, with the same core that runs it in
          production, and every response reports what it actually used.
        </p>
      </Section>

      <Section
        title="Agents fix their own mistakes"
        panel={
          <Replay
            title="aleph · an agent at work"
            lines={AGENT}
            caption="A small, closed vocabulary and precise errors: an agent can check its own work."
          />
        }
      >
        <p>
          Coding agents struggle most with the big picture: architecture, infrastructure, and which
          library to reach for. In Aleph those choices don’t exist. There’s one vocabulary, and it’s
          typed.
        </p>
        <p>
          When an agent writes something too expensive or unsafe, the checker says exactly what and
          where, and the agent can fix it before anyone reviews it.
        </p>
      </Section>

      <section className="closing">
        <h2>Build a first app in five minutes</h2>
        <p>Write one file, check it, test it, and serve it locally.</p>
        <div className="closing-links">
          <Link href="/docs/quickstart/" className="button">
            Start the quickstart
          </Link>
          <Link href="/docs/why-declarative/" className="button button-secondary">
            Why declarative
          </Link>
        </div>
      </section>
    </main>
  );
}
