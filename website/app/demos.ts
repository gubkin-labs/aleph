import type { Step } from "@/components/Demo";

/**
 * The homepage demos. Every scene uses one small notes app, and every line of CLI output is in
 * the CLI's real format and wording, run against that app; scenes only leave lines out.
 */

const ROUTES = [
  'a.route("GET /notes")',
  "  .query({ cursor: t.cursor() })",
  '  .mcp("list_notes")',
  "  .handle(($, { query }) =>",
  "    Note.page({ prefix: [$.caller], limit: 20, after: query.cursor }));",
];

const TABLE_HEAD =
  "route         reads  writes     ops    bytes read  effects   calls         memory";
const GET_ROW = "get_notes   20 (8%)  0 (0%)  6 (0%)  67.6 KB (3%)   0 (0%)  0 (0%)  199.3 KB (2%)";
const POST_ROW =
  "post_notes   1 (0%)  1 (2%)  7 (0%)   3.4 KB (0%)   0 (0%)  0 (0%)    6.8 KB (0%)";

export const cost = {
  start: 27,
  code: ROUTES,
  steps: [
    { cmd: "$ aleph check app.ts" },
    {
      out: [
        { t: "✔ valid: worst-case cost (share of the limit)", k: "ok" },
        { t: "" },
        { t: TABLE_HEAD, k: "dim", table: true },
      ],
    },
    {
      out: [
        { t: GET_ROW, table: true },
        { t: POST_ROW, table: true },
      ],
    },
    { pause: 700 },
    { hl: [31], mark: "limit: 20" },
    { termHl: [4] },
  ] satisfies Step[],
};

const ENTITY = [
  'const Note = a.entity("Note", {',
  '  key: ["ownerId", "noteId"],',
  "  fields: {",
  "    ownerId: t.string().max(64),",
  "    noteId: t.id().generated(),",
  "    text: t.string().min(1).max(500),",
  "    pinned: t.bool().default(false),",
  "  },",
  "});",
];

export const evolve = {
  start: 5,
  code: ENTITY,
  steps: [
    { edit: 10, from: "500", to: "200" },
    { insert: 11, text: "    color: t.string().max(16).optional()," },
    { cmd: "$ aleph check app.ts --against deployed.json" },
    { hl: [10], tone: "err", mark: "max(200)" },
    {
      out: [
        {
          t: "✖ entities.Note.fields.text: string length 1-500 narrowed to 1-200; existing values may not fit",
          k: "err",
        },
        { t: "✔ entities.Note.fields.color: new field color", k: "ok" },
        { t: "" },
        {
          t: "✖ not deployable: breaking changes need a deprecation in the deployed manifest first",
          k: "err",
        },
      ],
    },
    { pause: 1600 },
    { hl: [] },
    { edit: 10, from: "200", to: "500" },
    { clear: true },
    { cmd: "$ aleph check app.ts --against deployed.json" },
    {
      out: [
        { t: "✔ entities.Note.fields.color: new field color", k: "ok" },
        { t: "" },
        { t: "✔ deployable", k: "ok" },
      ],
    },
  ] satisfies Step[],
  // Without motion, show the moment the breaking change is caught.
  still: 5,
};

const EXAMPLE = [
  'a.route("POST /notes")',
  "  .body({ text: t.string().min(1).max(500) })",
  "  .status(201)",
  "  .example({",
  '    name: "creates a note",',
  '    as: { userId: "ada" },',
  '    request: { body: { text: "  hello  " } },',
  '    expect: { status: 201, body: { text: "hello" } },',
  "  })",
  "  .handle(($, { body }) =>",
  "    Note.insert({ ownerId: $.caller, text: body.text.trim() }));",
];

export const examples = {
  start: 15,
  code: EXAMPLE,
  steps: [
    { hl: [18, 19, 20, 21, 22, 23] },
    { cmd: "$ aleph test app.ts" },
    {
      out: [{ t: "✔ post_notes: creates a note", k: "ok" }, { t: "" }, { t: "1 passed, 0 failed" }],
    },
    { pause: 900 },
    { hl: [] },
    { cmd: "$ aleph dev app.ts --watch" },
    { out: [{ t: "aleph dev: serving app.ts on http://127.0.0.1:3000", k: "dim" }] },
    { pause: 300 },
    { out: [{ t: "POST /notes → 201 post_notes (reads 1, writes 1, ops 7) 1.3ms", k: "dim" }] },
    { pause: 300 },
    { out: [{ t: "GET /notes → 200 get_notes (reads 1, writes 0, ops 6) 0.46ms", k: "dim" }] },
  ] satisfies Step[],
};

export const agent = {
  start: 27,
  code: ROUTES,
  steps: [
    { agent: "you   ▸ Show more notes per page." },
    { edit: 31, from: "20", to: "5000" },
    { cmd: "$ aleph check app.ts" },
    { hl: [30, 31], tone: "err", mark: "limit: 5000" },
    {
      out: [
        { t: "✖ over_budget at app.ts:30:4 (routes[1].plan)", k: "err" },
        {
          t: '  route "get_notes" can use up to 5000 reads in the worst case, over the limit of 250. Lower the bounds that drive it (query limits, list sizes) or split the work.',
        },
        { t: "" },
        { t: "3 problems" },
      ],
    },
    { pause: 1200 },
    {
      agent:
        "agent ▸ 5000 is over the read budget of 250. Using 100 per page; the cursor gets the rest.",
    },
    { hl: [] },
    { edit: 31, from: "5000", to: "100" },
    { clear: true },
    { cmd: "$ aleph check app.ts" },
    {
      out: [
        { t: "✔ valid: worst-case cost (share of the limit)", k: "ok" },
        { t: "" },
        {
          t: "route           reads  writes     ops      bytes read  effects   calls         memory",
          k: "dim",
          table: true,
        },
        {
          t: "get_notes   100 (40%)  0 (0%)  6 (0%)  338.1 KB (17%)   0 (0%)  0 (0%)  740.4 KB (9%)",
          hl: true,
          table: true,
        },
        {
          t: "post_notes     1 (0%)  1 (2%)  7 (0%)     3.4 KB (0%)   0 (0%)  0 (0%)    6.8 KB (0%)",
          table: true,
        },
      ],
    },
  ] satisfies Step[],
};
