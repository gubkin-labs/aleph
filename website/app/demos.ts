import type { Step } from "@/components/Demo";

/**
 * The homepage demos. Every scene uses one small blog app (posts and their comments), and every
 * line of CLI output is in the CLI's real format and wording, run against that app; scenes only
 * leave lines out. The code lines keep the app's real line numbers, so locations in errors match.
 */

const LIST_POSTS = [
  'a.route("GET /posts")',
  "  .handle(() =>",
  "    Post.query({ limit: 20 }).map((post) => ({",
  "      post,",
  "      comments: Comment.query({ prefix: [post.postId], limit: 5 }),",
  "    })));",
];

const TABLE_HEAD =
  "route            reads  writes        ops      bytes read  effects   calls         memory";
const ADD_ROW =
  "add_comment     1 (0%)  1 (2%)     8 (0%)     3.5 KB (0%)   1 (2%)  0 (0%)    7.1 KB (0%)";
const CREATE_ROW =
  "create_post     1 (0%)  1 (2%)  489 (24%)     3.1 KB (0%)   0 (0%)  0 (0%)   34.3 KB (0%)";
const GET_ROW =
  "get_posts    120 (48%)  0 (0%)   187 (9%)  416.1 KB (20%)   0 (0%)  0 (0%)  1.28 MB (16%)";

const overBudget = (route: string, line: number, n: number, what: string, limit: number) => [
  {
    t: `✖ over_budget at app.ts:${line}:4 (routes[${route === "get_posts" ? 0 : 1}].plan)`,
    k: "err" as const,
  },
  {
    t: `  route "${route}" can use up to ${n} ${what} in the worst case, over the limit of ${limit}. Lower the bounds that drive it (query limits, list sizes) or split the work.`,
  },
];

export const cost = {
  start: 24,
  code: LIST_POSTS,
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
        { t: ADD_ROW, table: true },
        { t: CREATE_ROW, table: true },
        { t: GET_ROW, table: true },
      ],
    },
    { pause: 700 },
    { hl: [26, 28], mark: ["limit: 20", "limit: 5"] },
    { termHl: [6] },
    { pause: 1800 },
    { hl: [] },
    { edit: 26, from: "20", to: "10000" },
    { clear: true },
    { cmd: "$ aleph check app.ts" },
    { hl: [26], tone: "err", mark: "limit: 10000" },
    { out: [...overBudget("get_posts", 25, 60000, "reads", 250), { t: "" }, { t: "4 problems" }] },
  ] satisfies Step[],
  // Without motion, show the cost table with its bounds lit.
  still: 7,
};

const CREATE_POST = [
  'a.route("POST /posts")',
  '  .id("create_post")',
  "  .body({",
  "    title: t.string().min(1).max(200),",
  "    tags: t.array(t.string().max(32), 10),",
  "  })",
  "  .status(201)",
  "  .handle((_, { body }) =>",
  "    Post.insert({",
  "      title: body.title,",
  "      tags: body.tags.filter((tag) =>",
  "        body.tags.filter((other) => other.eq(tag)).length.eq(1)),",
  "    }));",
];

export const loop = {
  start: 31,
  code: CREATE_POST,
  steps: [
    { hl: [41, 42], mark: ["filter", "filter"] },
    { cmd: "$ aleph check app.ts" },
    {
      out: [
        { t: "✔ valid: worst-case cost (share of the limit)", k: "ok" },
        { t: "" },
        { t: TABLE_HEAD, k: "dim", table: true },
        { t: CREATE_ROW, table: true, hl: true },
      ],
    },
    { pause: 1400 },
    { hl: [] },
    { edit: 35, from: "32), 10)", to: "32), 100)" },
    { clear: true },
    { cmd: "$ aleph check app.ts" },
    { hl: [35], tone: "err", mark: "100)" },
    { out: [...overBudget("create_post", 38, 40809, "ops", 2000), { t: "" }, { t: "1 problem" }] },
  ] satisfies Step[],
};

const COMMENT = [
  'const Comment = a.entity("Comment", {',
  '  key: ["postId", "commentId"],',
  "  fields: {",
  "    postId: t.string().max(26),",
  "    commentId: t.id().generated(),",
  "    authorId: t.string().max(64),",
  "    text: t.string().min(1).max(500),",
  "  },",
  "});",
];

export const evolve = {
  start: 14,
  code: COMMENT,
  steps: [
    { edit: 20, from: "500", to: "200" },
    { insert: 20, text: "    likes: t.int().min(0).default(0)," },
    { cmd: "$ aleph check app.ts --against deployed.json" },
    { hl: [20], tone: "err", mark: "max(200)" },
    {
      out: [
        {
          t: "✖ entities.Comment.fields.text: string length 1-500 narrowed to 1-200; existing values may not fit",
          k: "err",
        },
        { t: "✔ entities.Comment.fields.likes: new field likes", k: "ok" },
        { t: "" },
        {
          t: "✖ not deployable: breaking changes need a deprecation in the deployed manifest first",
          k: "err",
        },
      ],
    },
    { pause: 1600 },
    { hl: [] },
    { edit: 20, from: "200", to: "500" },
    { clear: true },
    { cmd: "$ aleph check app.ts --against deployed.json" },
    {
      out: [
        { t: "✔ entities.Comment.fields.likes: new field likes", k: "ok" },
        { t: "" },
        { t: "✔ deployable", k: "ok" },
      ],
    },
  ] satisfies Step[],
  // Without motion, show the moment the breaking change is caught.
  still: 5,
};

const MORE = [
  'a.onInsert("moderate", Comment, ($, { payload }) => {',
  "  const comment = payload.after;",
  '  const verdict = moderation.call("check", { text: comment.text });',
  "  $.if(verdict.spam)",
  "    .then(() => comment.delete())",
  "    .else(() =>",
  "      live.publish({ postId: comment.postId }, { text: comment.text }));",
  "});",
  "",
  'a.every("weeklyDigest", "mon 09:00", ($) => {',
  '  const posts = Post.query({ limit: 10, order: "desc" });',
  '  email.send("send", {',
  '    to: "readers@example.com",',
  "    subject: $.fmt`${posts.length} new posts`,",
  "  });",
  "});",
];

export const more = {
  start: 76,
  code: MORE,
  steps: [
    { hl: [76], mark: "a.onInsert" },
    { pause: 900 },
    { hl: [78], mark: "moderation.call" },
    { pause: 900 },
    { hl: [82], mark: "live.publish" },
    { pause: 900 },
    { hl: [85], mark: 'a.every("weeklyDigest", "mon 09:00"' },
    { pause: 900 },
    { hl: [87], mark: "email.send" },
    { pause: 900 },
    { hl: [] },
    { cmd: "$ aleph check app.ts" },
    {
      out: [
        { t: "✔ valid: worst-case cost (share of the limit)", k: "ok" },
        { t: "" },
        {
          t: "handler         reads  writes      ops    bytes read  effects    calls        memory",
          k: "dim",
          table: true,
        },
        {
          t: "moderate       1 (0%)  1 (1%)  14 (0%)   3.5 KB (0%)   1 (1%)  1 (20%)     23 B (0%)",
          table: true,
        },
        {
          t: "weeklyDigest  10 (2%)  0 (0%)  11 (0%)  31.4 KB (0%)   1 (1%)   0 (0%)  31.4 KB (0%)",
          table: true,
        },
      ],
    },
    { termHl: [4, 5] },
  ] satisfies Step[],
};

export const agent = {
  start: 24,
  code: LIST_POSTS,
  steps: [
    { agent: "you   ▸ Show every comment under each post." },
    { edit: 28, from: "limit: 5 ", to: "limit: 1000 " },
    { cmd: "$ aleph check app.ts" },
    { hl: [26, 28], tone: "err", mark: ["limit: 20", "limit: 1000"] },
    { out: [...overBudget("get_posts", 25, 20020, "reads", 250), { t: "" }, { t: "3 problems" }] },
    { pause: 1200 },
    {
      agent:
        "agent ▸ 20 posts × 1000 comments is over the read budget of 250. Showing 10 per post; the full thread can page in on demand.",
    },
    { hl: [] },
    { edit: 28, from: "limit: 1000 ", to: "limit: 10 " },
    { clear: true },
    { cmd: "$ aleph check app.ts" },
    {
      out: [
        { t: "✔ valid: worst-case cost (share of the limit)", k: "ok" },
        { t: "" },
        { t: TABLE_HEAD, k: "dim", table: true },
        {
          t: "get_posts    220 (88%)  0 (0%)   187 (9%)  769.6 KB (38%)   0 (0%)  0 (0%)  2.32 MB (29%)",
          hl: true,
          table: true,
        },
      ],
    },
  ] satisfies Step[],
};
