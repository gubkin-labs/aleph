import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { ReactNode } from "react";
import { usage } from "@/app/demos";
import { Demo, type Step } from "@/components/Demo";
import { NodePane } from "@/components/NodePane";
import { Playground, type PlaygroundTab } from "@/components/Playground";
import {
  BoundedAleph,
  BoundedNode,
  ExactlyAleph,
  ExactlyNode,
  LessAleph,
  LessNode,
  PrivateAleph,
  PrivateNode,
} from "@/components/PlaygroundScenes";
import { scriptMs, tabMs } from "@/lib/script";

/**
 * The tabs under the hero. The Aleph side of every tab is read from a real app in
 * `playground-apps/` (checked by `test/playground.test.ts`). The ribbon across the Aleph code is
 * what the checker says about it; the Node side is typical Express code, condensed, whose ribbon
 * is amber because nothing stops it.
 */

function source(app: string, from: number, to: number) {
  const lines = readFileSync(join(process.cwd(), "playground-apps", `${app}.ts`), "utf8").split(
    "\n",
  );
  return lines.slice(from - 1, to);
}

// ─── 1. Bounded ─────────────────────────────────────────────────────────────────

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

// ─── 2. Exactly once ────────────────────────────────────────────────────────────

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

// ─── 3. Private by default ──────────────────────────────────────────────────────

const PRIVATE_NODE = [
  'app.use("/orgs", requireAuth);',
  "",
  "// added later, outside /orgs",
  'app.get("/invoices", async (req, res) => {',
  '  const { rows } = await db.query("SELECT * FROM invoices LIMIT 20");',
  "  res.json(rows);",
  "});",
];
const priv = {
  start: 14,
  code: source("private", 14, 20),
  steps: [
    { hl: [20], mark: ".handle" },
    { pause: 1400 },
    { hl: [16, 18], tone: "ok" },
    { verdict: "accepted" },
  ] satisfies Step[],
};

// ─── 4. Less to write ───────────────────────────────────────────────────────────

const LESS_NODE = [
  'import express from "express";',
  'import { rateLimit } from "express-rate-limit";',
  'import { z } from "zod";',
  "",
  "const limiter = rateLimit({ windowMs: 60_000, limit: 10, keyGenerator: (r) => r.user.id });",
  "const Order = z.object({ item: z.string().min(1).max(64) });",
  "",
  'app.post("/orders", limiter, async (req, res) => {',
  '  const key = req.header("Idempotency-Key");',
  "  const body = Order.safeParse(req.body);",
  "  if (!body.success) return res.status(400).json(body.error);",
  "  // two retries can both get past this check",
  // biome-ignore lint/suspicious/noTemplateCurlyInString: this is displayed source code
  "  const seen = await redis.get(`idem:${req.user.id}:${key}`);",
  "  if (seen) return res.status(201).json(JSON.parse(seen));",
  "  const { rows } = await db.query(",
  '    "INSERT INTO orders (customer_id, item) VALUES ($1, $2) RETURNING *",',
  "    [req.user.id, body.data.item],",
  "  );",
  // biome-ignore lint/suspicious/noTemplateCurlyInString: this is displayed source code
  "  await redis.set(`idem:${req.user.id}:${key}`, JSON.stringify(rows[0]), { EX: 86400 });",
  "  res.status(201).json(rows[0]);",
  "});",
  "",
  'app.post("/webhooks/stripe", express.raw({ type: "application/json" }), async (req, res) => {',
  "  let event;",
  "  try {",
  '    event = stripe.webhooks.constructEvent(req.body, req.header("stripe-signature"), secret);',
  "  } catch {",
  "    return res.status(401).end();",
  "  }",
  // biome-ignore lint/suspicious/noTemplateCurlyInString: this is displayed source code
  '  const first = await redis.set(`evt:${event.id}`, "1", { NX: true, EX: 259200 });',
  "  if (!first) return res.json({ received: true }); // a duplicate delivery",
  "  res.json({ received: true });",
  "});",
];
const less = {
  start: 15,
  code: source("less", 15, 27),
  steps: [
    { hl: [16, 17, 23, 24], mark: [".idempotent", ".rateLimit", "verify.stripe", "dedupe"] },
    { pause: 1400 },
    {
      hl: [16, 17, 23, 24],
      tone: "ok",
      mark: [".idempotent", ".rateLimit", "verify.stripe", "dedupe"],
    },
    { verdict: "accepted" },
  ] satisfies Step[],
};

// ─── The tabs ───────────────────────────────────────────────────────────────────

/** About how long the scenes take to play. */
const SCENE_MS = 5500;

type Script = { start: number; code: string[]; steps: Step[] };

function aleph(demo: Script, scene: ReactNode) {
  return (
    <>
      {scene}
      <Demo title="aleph · app.ts" start={demo.start} code={demo.code} steps={demo.steps} noTerm />
    </>
  );
}

function node(code: string[], flag: number[], scene: ReactNode) {
  return (
    <>
      {scene}
      <NodePane title="typical node + express" file="server.ts" code={code} flag={flag} />
    </>
  );
}

export function PlaygroundSection() {
  const tabs: PlaygroundTab[] = [
    {
      id: "bounded",
      label: "Bounded",
      ms: tabMs(scriptMs(bounded.steps), 8000),
      aleph: aleph(bounded, <BoundedAleph />),
      other: node(BOUNDED_NODE, [2, 5], <BoundedNode />),
    },
    {
      id: "exactly-once",
      label: "Exactly once",
      ms: tabMs(scriptMs(exactly.steps), SCENE_MS),
      aleph: aleph(exactly, <ExactlyAleph />),
      other: node(EXACTLY_NODE, [6], <ExactlyNode />),
    },
    {
      id: "private",
      label: "Private by default",
      ms: tabMs(scriptMs(priv.steps), SCENE_MS),
      aleph: aleph(priv, <PrivateAleph />),
      other: node(PRIVATE_NODE, [4], <PrivateNode />),
    },
    {
      id: "less",
      label: "Less to write",
      ms: tabMs(scriptMs(less.steps), SCENE_MS),
      aleph: aleph(less, <LessAleph />),
      other: node(LESS_NODE, [12, 13], <LessNode />),
    },
  ];
  return <Playground tabs={tabs} />;
}
