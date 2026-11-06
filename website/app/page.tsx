import Link from "next/link";
import type { ReactNode } from "react";
import { Demo, Panel } from "@/components/Demo";
import { Install } from "@/components/Install";
import { GeneratedScene, IsolationScene, SharedScene } from "@/components/Scenes";
import { agent, cost, evolve, examples } from "./demos";

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
          Cheap to start. <em>Safe at scale.</em>
        </h1>
        <p className="lead">
          Aleph is a declarative language for backends. Every request’s cost is known before it
          runs, so apps can safely share servers and databases, and every change is checked before
          it ships.
        </p>
        <Install />
      </section>

      <section className="audiences">
        <div>
          <h3>For a startup</h3>
          <ul>
            <li>
              No <strong>$100 a month</strong> idle stack before your first user. Apps share
              capacity and pay for the requests they serve.
            </li>
            <li>Nothing to set up: no servers, pipelines, or libraries to keep upgraded.</li>
            <li>Your coding agent can build the backend and check its own work.</li>
          </ul>
        </div>
        <div>
          <h3>For an enterprise</h3>
          <ul>
            <li>Run hundreds of services on shared infrastructure without noisy neighbors.</li>
            <li>
              Every route’s cost is bounded and every schema change is classified before deploy.
            </li>
            <li>No third-party code runs in production, and apps are isolated by construction.</li>
          </ul>
        </div>
      </section>

      <Section
        title="Know the cost before you deploy"
        panel={
          <Demo
            title="worst case of every route"
            {...cost}
            caption="The query limit on line 31 is why get_notes reads at most 20 rows."
          />
        }
      >
        <p>
          You describe routes, data, and logic in ordinary TypeScript. Aleph records the description
          instead of running it, and works out the most each route can ever read, write, compute,
          and hold in memory.
        </p>
        <p>
          A route that could go over its limit is rejected before it ships, with a message saying
          which bound to change. The same limits are metered while it runs, so a budget is a
          guarantee, not an estimate.
        </p>
      </Section>

      <Section
        title="Safe to share, so cheap to run"
        panel={
          <Panel
            title="many apps, one runtime"
            caption="Each request stays under its own ceiling, so no app can slow down the rest."
          >
            <SharedScene />
          </Panel>
        }
      >
        <p>
          A backend usually gets its own server, database, and cache, sized for its busiest hour and
          idle the rest of the day. It needs them to itself because, in a shared setup, one runaway
          query slows everyone down.
        </p>
        <p>
          In Aleph that can’t happen. So a runtime can pack many apps into the same processes and
          the same database, the way static sites share a CDN. A startup stops paying for idle
          capacity; a platform team consolidates hundreds of internal services.{" "}
          <Link href="/docs/shared-infrastructure/">How sharing works</Link>
        </p>
      </Section>

      <Section
        title="Isolated by construction"
        panel={
          <Panel
            title="one database, many apps"
            caption="The core builds every key inside the app's own prefix. Plans never see raw keys."
          >
            <IsolationScene />
          </Panel>
        }
      >
        <p>
          Sharing a database is only acceptable if one app can never read another’s data. In Aleph
          that isn’t a permission check that could be misconfigured: there’s no expression that
          names another app’s keys, so cross-app access can’t be written at all.
        </p>
        <p>
          And no author code runs in production. A manifest is data, interpreted by one small,
          audited core, so there’s no sandbox to escape and no dependency tree to patch.{" "}
          <Link href="/docs/running/isolation/">Tenants and isolation</Link>
        </p>
      </Section>

      <Section
        title="Nothing to operate"
        panel={
          <Panel
            title="one description, everything derived"
            caption="The manifest is the only thing that ships. Everything else comes from it."
          >
            <GeneratedScene />
          </Panel>
        }
      >
        <p>
          The app compiles into a manifest: a versioned description that any Aleph runtime can run.
          There are no servers to size, no Dockerfiles or deploy pipelines, and no ORM, queue
          client, or auth library to wire up and keep upgraded.
        </p>
        <p>
          The same description produces an OpenAPI spec, a typed client, and MCP tools for agents,
          so the docs, the client, and the backend can’t drift apart.
        </p>
      </Section>

      <Section
        title="Change it without fear"
        panel={
          <Demo
            title="compare with what's deployed"
            {...evolve}
            caption="Narrowing a field could break stored rows, so it's blocked. Adding one is fine."
          />
        }
      >
        <p>
          Every request runs as one serializable transaction, and events, emails, and jobs go out
          exactly once, only after it commits. There are no half-written states to clean up.
        </p>
        <p>
          When the schema changes, Aleph compares it with what’s deployed and classifies every
          change as safe, needing a backfill, or breaking. Breaking changes are blocked until
          they’re deprecated properly, whoever wrote them.
        </p>
      </Section>

      <Section
        title="Examples are the tests"
        panel={
          <Demo
            title="test, then serve locally"
            {...examples}
            caption="Each example runs on a fresh store and checks the route stayed within its bound."
          />
        }
      >
        <p>
          Write examples next to a route: given this data and this request, expect this response.
          They document the route and run as its tests.
        </p>
        <p>
          <code>aleph dev</code> serves the app on your machine with the same core that runs it in
          production, and every response reports what it actually used.
        </p>
      </Section>

      <Section
        title="Agents fix their own mistakes"
        panel={
          <Demo
            title="an agent at work"
            {...agent}
            caption="The error names the route, the line, and the bound to lower. The agent does the rest."
          />
        }
      >
        <p>
          Coding agents struggle most with the big picture: architecture, infrastructure, and which
          library to reach for. In Aleph those choices don’t exist. There’s one small, typed
          vocabulary.
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
