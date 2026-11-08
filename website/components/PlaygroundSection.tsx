import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { ReactNode } from "react";
import { usage } from "@/app/demos";
import { Demo, type Step } from "@/components/Demo";
import { NodePane } from "@/components/NodePane";
import { Playground, type PlaygroundTab } from "@/components/Playground";
import {
  AgnosticAleph,
  AgnosticTypical,
  ExactlyAleph,
  ExactlyNode,
  GuardrailsScene,
  InfraScene,
  RustAleph,
  RustTypical,
  SharedAleph,
  SharedTypical,
} from "@/components/PlaygroundScenes";
import { scriptMs, tabMs } from "@/lib/script";

/**
 * The tabs under the hero. The Aleph code in every tab is read from a real app in
 * `playground-apps/` (checked by `test/playground.test.ts`). The ribbon across the Aleph code is
 * what the checker says about it; the Node code is typical Express, condensed, whose ribbon is
 * amber because nothing stops it.
 */

function source(app: string, from: number, to: number) {
  const lines = readFileSync(join(process.cwd(), "playground-apps", `${app}.ts`), "utf8").split(
    "\n",
  );
  return lines.slice(from - 1, to);
}

/**
 * Tabs that share one scene over both code panes play a typical backend first (about 5 seconds),
 * so the Aleph code waits that long before it starts to change.
 */
const AFTER_TYPICAL: Step = { pause: 5000 };

// ─── 1. Infra protection ────────────────────────────────────────────────────────

const BOUNDED_NODE = [
  'app.get("/posts", async (req, res) => {',
  '  const { rows: posts } = await db.query("SELECT * FROM posts");',
  "  const out = [];",
  "  for (const post of posts) {",
  '    const { rows } = await db.query("SELECT * FROM comments WHERE post_id = $1", [post.id]);',
  "    out.push({ post, comments: rows });",
  "  }",
  "  res.json(out);",
  "});",
];
const bounded = {
  start: usage.start,
  code: usage.code,
  steps: [
    AFTER_TYPICAL,
    { pause: 500 },
    { edit: 27, from: "limit: 20", to: "limit: 10000" },
    { hl: [27], tone: "err", mark: "limit: 10000" },
    { verdict: "rejected" },
    { pause: 1600 },
    { hl: [] },
    { verdict: null },
    { edit: 27, from: "limit: 10000", to: "limit: 20" },
    { hl: [27], tone: "ok", mark: "limit: 20" },
    { verdict: "accepted" },
  ] satisfies Step[],
};

// ─── 2. Builtin guardrails ──────────────────────────────────────────────────────

const GUARDRAILS_NODE = [
  'app.post("/posts", async (req, res) => {',
  "  const { title, tags } = req.body;",
  "  const unique = tags.filter(",
  "    (tag) => tags.filter((other) => other === tag).length === 1,",
  "  );",
  "  const { rows } = await db.query(",
  '    "INSERT INTO posts (title, tags) VALUES ($1, $2) RETURNING *",',
  "    [title, unique],",
  "  );",
  "  res.status(201).json(rows[0]);",
  "});",
];
const guardrails = {
  start: 14,
  code: source("guardrails", 14, 26),
  steps: [
    AFTER_TYPICAL,
    { pause: 500 },
    { edit: 18, from: "32), 10)", to: "32), 100)" },
    { hl: [18], tone: "err", mark: "100)" },
    { verdict: "rejected" },
    { pause: 1600 },
    { hl: [] },
    { verdict: null },
    { edit: 18, from: "32), 100)", to: "32), 10)" },
    { hl: [18], tone: "ok", mark: "10)" },
    { verdict: "accepted" },
  ] satisfies Step[],
};

// ─── 3. Exactly once by default ─────────────────────────────────────────────────

const EXACTLY_NODE = [
  'app.post("/orders", async (req, res) => {',
  "  const { rows } = await db.query(",
  '    "INSERT INTO orders (customer_id, item) VALUES ($1, $2) RETURNING *",',
  "    [req.user.id, req.body.item],",
  "  );",
  '  await mailer.send({ to: req.user.email, subject: "Your receipt" });',
  "  res.status(201).json(rows[0]);",
  "});",
];
const exactly = {
  start: 21,
  code: source("exactly", 21, 31),
  steps: [
    { hl: [25, 26], mark: ["Order.insert", "email.send"] },
    { pause: 1400 },
    { hl: [25, 26], tone: "ok", mark: ["Order.insert", "email.send"] },
    { verdict: "accepted" },
  ] satisfies Step[],
};

// ─── 5. Write in TS, execute in Rust ────────────────────────────────────────────

const RUNTIME_NODE = [
  'app.get("/posts/:postId", async (req, res) => {',
  '  const { rows } = await db.query("SELECT * FROM posts WHERE id = $1", [req.params.postId]);',
  "  if (rows.length === 0) return res.status(404).end();",
  "  res.json(rows[0]);",
  "});",
];
const runtime = { start: 5, code: source("runtime", 5, 15), steps: [] as Step[] };

// ─── Layout ─────────────────────────────────────────────────────────────────────

type Pos = { c: 1 | 2 | "w"; r: 1 | 2 };
function Cell({ c, r, children }: Pos & { children: ReactNode }) {
  return <div className={`pg-c${c} pg-r${r}`}>{children}</div>;
}

function alephCode(demo: { start: number; code: string[]; steps: Step[] }) {
  return (
    <Demo title="aleph · app.ts" start={demo.start} code={demo.code} steps={demo.steps} noTerm />
  );
}

function nodeCode(code: string[], flag: number[], ribbon = true) {
  return (
    <NodePane
      title="typical node + express"
      file="server.ts"
      code={code}
      flag={flag}
      ribbon={ribbon}
    />
  );
}

/** One scene over both code panes. */
function sharedScene(scene: ReactNode, left: ReactNode, right: ReactNode) {
  return (
    <>
      <Cell c="w" r={1}>
        {scene}
      </Cell>
      <Cell c={1} r={2}>
        {left}
      </Cell>
      <Cell c={2} r={2}>
        {right}
      </Cell>
    </>
  );
}

/** A scene over each code pane. */
function ownScenes(sceneL: ReactNode, left: ReactNode, sceneR: ReactNode, right: ReactNode) {
  return (
    <>
      <Cell c={1} r={1}>
        {sceneL}
      </Cell>
      <Cell c={1} r={2}>
        {left}
      </Cell>
      <Cell c={2} r={1}>
        {sceneR}
      </Cell>
      <Cell c={2} r={2}>
        {right}
      </Cell>
    </>
  );
}

/** Two scenes and no code, one over the other. */
function stacked(top: ReactNode, bottom: ReactNode) {
  return (
    <>
      <Cell c="w" r={1}>
        {top}
      </Cell>
      <Cell c="w" r={2}>
        {bottom}
      </Cell>
    </>
  );
}

export function PlaygroundSection() {
  const tabs: PlaygroundTab[] = [
    {
      id: "agnostic",
      label: "Platform agnostic",
      layout: "stack",
      ms: tabMs(9000),
      body: stacked(<AgnosticAleph />, <AgnosticTypical />),
    },
    {
      id: "infra",
      label: "Infra protection",
      layout: "code",
      ms: tabMs(scriptMs(bounded.steps), 12500),
      body: sharedScene(<InfraScene />, alephCode(bounded), nodeCode(BOUNDED_NODE, [2, 5])),
    },
    {
      id: "guardrails",
      label: "Builtin guardrails",
      layout: "code",
      ms: tabMs(scriptMs(guardrails.steps), 12500),
      body: sharedScene(
        <GuardrailsScene />,
        alephCode(guardrails),
        nodeCode(GUARDRAILS_NODE, [3, 4]),
      ),
    },
    {
      id: "exactly-once",
      label: "Exactly once by default",
      layout: "code",
      ms: tabMs(scriptMs(exactly.steps), 7000),
      body: ownScenes(
        <ExactlyAleph />,
        alephCode(exactly),
        <ExactlyNode />,
        nodeCode(EXACTLY_NODE, [6]),
      ),
    },
    {
      id: "shared",
      label: "Shared infra & reduced costs",
      layout: "stack",
      ms: tabMs(7000),
      body: stacked(<SharedAleph />, <SharedTypical />),
    },
    {
      id: "rust",
      label: "Write in TS, execute in Rust",
      layout: "code",
      ms: tabMs(8000),
      body: ownScenes(
        <RustAleph />,
        alephCode(runtime),
        <RustTypical />,
        nodeCode(RUNTIME_NODE, [], false),
      ),
    },
  ];
  return <Playground tabs={tabs} />;
}
