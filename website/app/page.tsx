import Link from "next/link";
import type { CSSProperties, ReactNode } from "react";
import { Demo, Panel } from "@/components/Demo";
import { Install } from "@/components/Install";
import {
  HowScene,
  IsolationScene,
  SharedScene,
  TradeoffScene,
  TransactionScene,
} from "@/components/Scenes";
import { agent, cost, evolve, loop, more } from "./demos";

/** What a backend usually needs and an Aleph author doesn't write. Each is struck out in turn. */
const HANDLED = [
  "servers",
  "scaling",
  "connection pools",
  "transactions",
  "retries",
  "exactly-once events",
  "input validation",
  "auth middleware",
  "migrations",
  "tenant isolation",
  "API docs",
  "client SDK",
];

function Section({
  eyebrow,
  panel,
  title,
  children,
}: {
  eyebrow?: string;
  panel: ReactNode;
  title: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="row">
      <div className="row-panel">{panel}</div>
      <div className="row-text">
        {eyebrow && <p className="eyebrow">{eyebrow}</p>}
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
          The future of generated code is <em>declarative.</em>
        </h1>
        <p className="lead">
          Aleph is a trusted, declarative language for server-side applications. You write the
          routes, access rules, and queries. Aleph takes care of everything else.
        </p>
        <ul className="handled" aria-label="Handled by Aleph">
          {HANDLED.map((h, i) => (
            <li key={h} style={{ "--i": i } as CSSProperties}>
              {h}
            </li>
          ))}
          <li className="handled-yours" style={{ "--i": HANDLED.length } as CSSProperties}>
            your business logic
          </li>
        </ul>
        <Install />
      </section>

      <Section
        eyebrow="Why Aleph"
        title={
          <>
            Less flexibility, <em>on purpose.</em>
          </>
        }
        panel={
          <Panel
            title="two roads to production"
            caption="General-purpose code steers around everything a backend needs. In Aleph, it’s built into the road."
          >
            <TradeoffScene />
          </Panel>
        }
      >
        <p>
          General-purpose languages let an app do anything. So every app needs its own servers, its
          own review, and its own guardrails, and someone has to set up and maintain all of it.
        </p>
        <p>
          Aleph makes the opposite trade. It gives up some flexibility for shared building blocks
          for what every backend needs, predictable behavior, and code that’s trusted by default.
          When agents write the code, that trade matters more than ever: a narrow language is easy
          to check and hard to get wrong. <Link href="/docs/#why-declarative">Why declarative</Link>
        </p>
      </Section>

      <Section
        eyebrow="Guardrails"
        title="No query can read too much"
        panel={
          <Demo
            title="worst case of every route"
            {...cost}
            caption="A page of 20 posts, 5 comments each, and a cursor for the next page: at most 120 reads. Ask for 10,000 at once and it won't build."
          />
        }
      >
        <p>
          Every query has a limit, and Aleph multiplies them out: a page of 20 posts with 5 comments
          each is at most 120 reads, before a single request runs.
        </p>
        <p>
          So the natural way to return a long list is the right one: a page and a cursor. Ask for
          everything at once and it’s rejected before it ships, with the line to change. The same
          limits are enforced while it runs, so a budget is a guarantee, not a guess.
        </p>
      </Section>

      <Section
        eyebrow="Guardrails"
        title="No loop can run away"
        panel={
          <Demo
            title="a loop inside a loop"
            {...loop}
            caption="Comparing every tag with every other: 10 tags is 489 ops. Allow 100 and it's 40,809, over the limit of 2,000."
          />
        }
      >
        <p>
          There’s no <code>while</code> loop and no recursion. Every loop runs over a list with a
          known maximum size, so even a loop inside a loop has a worst case Aleph can count.
        </p>
        <p>
          That’s why one app can’t burn the CPU that other apps share: code that could, doesn’t
          compile.
        </p>
      </Section>

      <Section
        eyebrow="Still flexible"
        title="Bounded, not basic"
        panel={
          <Demo
            title="more than routes"
            {...more}
            rows={18}
            caption="An HTTP API, email, a live channel, a trigger, and a weekly schedule, all in the same app.ts, each with a worst case."
          />
        }
      >
        <p>
          The limits are on cost, not on what you can build. Beyond routes and queries, the same
          language calls external APIs, reacts to changes, pushes updates to clients in real time,
          sends email, and runs schedules, queues, webhooks, and multi-step workflows that can wait
          for days.
        </p>
        <p>
          Every one of them is checked the same way: triggers and schedules get a worst-case cost,
          and their effects go out only after they commit.{" "}
          <Link href="/docs/background-work/">Background work and workflows</Link>
        </p>
      </Section>

      <Section
        eyebrow="How it works"
        title="You write the logic. The platform runs it."
        panel={
          <Panel
            title="from app.ts to production"
            caption="The manifest is the only thing that ships. Everything else is derived from it."
          >
            <HowScene />
          </Panel>
        }
      >
        <ol className="steps">
          <li>
            <strong>Describe your app in TypeScript.</strong> Routes, who can call them, the data,
            and the queries. No server, ORM, or auth library to set up.
          </li>
          <li>
            <strong>Aleph compiles it into a manifest.</strong> Data, not code: exactly what each
            route can do, and the most it can ever cost.
          </li>
          <li>
            <strong>A platform runs the manifest.</strong> One small, audited core interprets it, so
            your code never runs in production.
          </li>
        </ol>
        <p>
          Because of those guardrails, many apps can run in the same process and share the same
          database, and a platform pays for capacity once instead of once per app.
        </p>
      </Section>

      <header className="group">
        <p className="eyebrow">Advantages</p>
        <h2>What the trade buys you</h2>
      </header>

      <Section
        title="Safe to share, so cheap to run"
        panel={
          <div className="panels">
            <Panel
              title="many apps, one runtime"
              caption="Each request stays under its own ceiling, so no app can slow down the rest."
            >
              <SharedScene />
            </Panel>
            <Panel
              title="one database, many apps"
              caption="Every key is built inside the app's own prefix. Plans never see raw keys."
            >
              <IsolationScene />
            </Panel>
          </div>
        }
      >
        <p>
          A backend usually gets its own server, database, and cache, sized for its busiest hour and
          idle the rest of the day. It needs them to itself because, in a shared setup, one runaway
          query or one bug can hurt everyone.
        </p>
        <p>
          Aleph rules both out. Every route has a bounded cost, and there’s no expression that names
          another app’s data, so cross-app access can’t be written at all. Apps can safely share
          servers and a database, and you stop paying for idle capacity.
        </p>
        <p>
          That changes the price, too. A platform doesn’t have to set aside servers and a database
          for every app it hosts, so it can offer apps built with Aleph much better pricing than a
          dedicated deployment. <Link href="/docs/shared-infrastructure/">How sharing works</Link>
        </p>
      </Section>

      <Section
        title="Correct by default"
        panel={
          <Panel
            title="every request is one transaction"
            caption="Effects are queued inside the transaction and sent only after it commits."
          >
            <TransactionScene />
          </Panel>
        }
      >
        <p>
          Every request runs as one serializable transaction. Events, emails, and jobs go out
          exactly once, and only after it commits.
        </p>
        <p>
          There are no half-written states to clean up, no retry logic to write, and no message that
          was sent for a change that never happened.
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
          Every schema change is compared with what’s deployed and classified as safe, needing a
          backfill, or breaking. Breaking changes are blocked until they’re deprecated properly,
          whoever wrote them.
        </p>
        <p>
          Additive changes ship right away. Anything that could break data already stored waits for
          a proper deprecation, so stored rows always fit the schema.
        </p>
      </Section>

      <Section
        title="Built for agents"
        panel={
          <Demo
            title="an agent at work"
            {...agent}
            caption="The error names the route, the line, and the bound to lower. The agent does the rest."
          />
        }
      >
        <p>
          A small, typed vocabulary means there are no architecture, infrastructure, or library
          choices for an agent to get wrong.
        </p>
        <p>
          When it does slip, the checker names the route, the line, and the bound, so the agent
          fixes its own mistake before anyone reviews it.
        </p>
      </Section>

      <section className="closing">
        <h2>Build a first app in five minutes</h2>
        <p>Write one file, check it, test it, and serve it locally.</p>
        <div className="closing-links">
          <Link href="/docs/quickstart/" className="button">
            Start the quickstart
          </Link>
          <Link href="/docs/#why-declarative" className="button button-secondary">
            Why declarative
          </Link>
        </div>
      </section>
    </main>
  );
}
