import type { Step } from "@/components/Demo";

/**
 * The homepage demos. Every scene uses one small blog app (posts and their comments), and every
 * line of CLI output is in the CLI's real format and wording, run against that app; scenes only
 * leave lines out. The code lines keep the app's real line numbers, so locations in errors match.
 */

const LIST_POSTS = [
  'a.route("GET /posts")',
  "  .query({ cursor: t.cursor() })",
  "  .handle((_, { query }) => {",
  "    const page = Post.page({ limit: 20, after: query.cursor });",
  "    return {",
  "      next: page.cursor,",
  "      posts: page.items.map((post) => ({",
  "        post,",
  "        comments: Comment.query({ prefix: [post.postId], limit: 5 }),",
  "      })),",
  "    };",
  "  });",
];

const TABLE_HEAD =
  "route           reads   writes         ops     bytes read   effects   calls        memory";
const LIMIT_ROW =
  "limit         250        50      2,000           2 MB         50        3        8 MB";
const ADD_ROW =
  "add_comment     1 <1%     1 2%       8 <1%     3.5 KB <1%      1 2%     0 –    7.1 KB <1%";
const CREATE_ROW =
  "create_post     1 <1%     1 2%     489 24%     3.1 KB <1%      0  –     0 –   34.3 KB <1%";
const GET_ROW =
  "get_posts     120 48%     0  –     192 10%   416.1 KB 20%      0  –     0 –   1.34 MB 17%";

const overBudget = (route: string, line: number, n: number, what: string, limit: number) => [
  {
    t: `✖ over_budget at app.ts:${line}:4 (routes[${route === "get_posts" ? 0 : 1}].plan)`,
    k: "err" as const,
  },
  {
    t: `  route "${route}" can use up to ${n} ${what} in the worst case, over the limit of ${limit}. Lower the bounds that drive it (query limits, list sizes) or split the work.`,
  },
];

export const usage = {
  start: 24,
  code: LIST_POSTS,
  steps: [
    { cmd: "$ aleph check app.ts" },
    {
      out: [
        { t: "✔ valid. The most each route and handler can use in one run:", k: "ok" },
        { t: "" },
        { t: TABLE_HEAD, k: "dim", table: true },
        { t: LIMIT_ROW, k: "dim", table: true },
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
    { hl: [25, 27, 32], mark: ["t.cursor()", "limit: 20", "limit: 5"] },
    { termHl: [7] },
    { pause: 1800 },
    { hl: [] },
    { edit: 27, from: "limit: 20", to: "limit: 10000" },
    { clear: true },
    { cmd: "$ aleph check app.ts" },
    { hl: [27], tone: "err", mark: "limit: 10000" },
    { out: [...overBudget("get_posts", 26, 60000, "reads", 250), { t: "" }, { t: "4 problems" }] },
  ] satisfies Step[],
  // Without motion, show the usage table with its bounds lit.
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
  start: 37,
  code: CREATE_POST,
  steps: [
    { hl: [47, 48], mark: ["filter", "filter"] },
    { cmd: "$ aleph check app.ts" },
    {
      out: [
        { t: "✔ valid. The most each route and handler can use in one run:", k: "ok" },
        { t: "" },
        { t: TABLE_HEAD, k: "dim", table: true },
        { t: LIMIT_ROW, k: "dim", table: true },
        { t: CREATE_ROW, table: true, hl: true },
      ],
    },
    { pause: 1400 },
    { hl: [] },
    { edit: 41, from: "32), 10)", to: "32), 100)" },
    { clear: true },
    { cmd: "$ aleph check app.ts" },
    { hl: [41], tone: "err", mark: "100)" },
    { out: [...overBudget("create_post", 44, 40809, "ops", 2000), { t: "" }, { t: "1 problem" }] },
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
  'import { app, t } from "@aleph-lang/sdk";',
  "",
  'const a = app("blog"); // the app everything is declared on',
  "",
  'const Post = a.entity("Post", {',
  '  key: ["postId"],',
  "  fields: {",
  "    postId: t.id().generated(),",
  "    title: t.string().min(1).max(200),",
  "    tags: t.array(t.string().max(32), 10),",
  "  },",
  "});",
  "",
  'const Comment = a.entity("Comment", {',
  '  key: ["postId", "commentId"],',
  "  fields: {",
  "    postId: t.string().max(26),",
  "    commentId: t.id().generated(),",
  "    authorId: t.string().max(64),",
  "    text: t.string().min(1).max(500),",
  "  },",
  "});",
  "",
  'a.route("GET /posts")',
  "  .query({ cursor: t.cursor() })",
  "  .handle((_, { query }) => {",
  "    const page = Post.page({ limit: 20, after: query.cursor });",
  "    return {",
  "      next: page.cursor,",
  "      posts: page.items.map((post) => ({",
  "        post,",
  "        comments: Comment.query({ prefix: [post.postId], limit: 5 }),",
  "      })),",
  "    };",
  "  });",
  "",
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
  "",
  'a.route("POST /posts/:postId/comments")',
  '  .id("add_comment")',
  "  .params({ postId: t.string().max(26) })",
  "  .body({ text: t.string().min(1).max(500) })",
  "  .status(201)",
  "  .handle(($, { params, body }) =>",
  "    Comment.insert({",
  "      postId: params.postId,",
  "      authorId: $.caller,",
  "      text: body.text.trim(),",
  "    }));",
  "",
  '// An external HTTP API. "call" waits for the answer.',
  "// Each call counts toward the limit.",
  'const moderation = a.integration("moderation", {',
  '  provider: "http",',
  "  operations: {",
  "    check: {",
  '      mode: "call",',
  "      input: { text: t.string().max(500) },",
  "      output: { spam: t.bool() },",
  "    },",
  "  },",
  "});",
  "",
  '// Email. "effect" is sent after commit,',
  "// exactly once.",
  'const email = a.integration("email", {',
  '  provider: "email",',
  "  operations: {",
  "    send: {",
  '      mode: "effect",',
  "      input: { to: t.string().max(200), subject: t.string().max(200) },",
  "    },",
  "  },",
  "});",
  "",
  "// A live channel that clients subscribe to, per post.",
  'const live = a.channel("comments", {',
  "  key: { postId: t.string().max(26) },",
  "  message: { text: t.string().max(500) },",
  "});",
  "",
  "// A trigger: runs after each new comment commits,",
  "// in the background. The request doesn't wait.",
  'a.onInsert("moderate", Comment, ($, { payload }) => {',
  "  const comment = payload.after;",
  '  const verdict = moderation.call("check", { text: comment.text });',
  "  // Delete spam; push everything else live.",
  "  $.if(verdict.spam)",
  "    .then(() => comment.delete())",
  "    .else(() =>",
  "      live.publish({ postId: comment.postId }, { text: comment.text }));",
  "});",
  "",
  "// A schedule: every Monday at 09:00.",
  'a.every("weeklyDigest", "mon 09:00", ($) => {',
  '  const posts = Post.query({ limit: 10, order: "desc" });',
  '  email.send("send", {',
  '    to: "readers@example.com",',
  // biome-ignore lint/suspicious/noTemplateCurlyInString: this is displayed source code
  "    subject: $.fmt`${posts.length} new posts`,",
  "  });",
  "});",
  "",
  "export default a;",
];

export const more = {
  start: 1,
  code: MORE,
  steps: [
    { hl: [3], mark: 'app("blog")' },
    { pause: 1000 },
    { hl: [65], mark: 'a.integration("moderation"' },
    { pause: 1000 },
    { hl: [78], mark: 'a.integration("email"' },
    { pause: 1000 },
    { hl: [89], mark: 'a.channel("comments"' },
    { pause: 1000 },
    { hl: [96], mark: "a.onInsert" },
    { pause: 900 },
    { hl: [98], mark: "moderation.call" },
    { pause: 900 },
    { hl: [103], mark: "live.publish" },
    { pause: 900 },
    { hl: [107], mark: 'a.every("weeklyDigest", "mon 09:00"' },
    { pause: 900 },
    { hl: [109], mark: "email.send" },
    { pause: 1000 },
    { hl: [] },
    { cmd: "$ aleph check app.ts" },
    {
      out: [
        { t: "✔ valid. The most each route and handler can use in one run:", k: "ok" },
        { t: "" },
        {
          t: "handler          reads    writes         ops    bytes read   effects   calls        memory",
          k: "dim",
          table: true,
        },
        {
          t: "limit          500       200       5,000          8 MB       200       5         32 MB",
          k: "dim",
          table: true,
        },
        {
          t: "moderate         1 <1%     1 <1%      14 <1%    3.5 KB <1%     1 <1%   1 20%      23 B <1%",
          table: true,
        },
        {
          t: "weeklyDigest    10  2%     0   –      11 <1%   31.4 KB <1%     1 <1%   0   –   31.4 KB <1%",
          table: true,
        },
      ],
    },
    { termHl: [5, 6] },
  ] satisfies Step[],
};

export const agent = {
  start: 24,
  code: LIST_POSTS,
  steps: [
    { agent: "you   ▸ Show every comment under each post." },
    { edit: 32, from: "limit: 5 ", to: "limit: 1000 " },
    { cmd: "$ aleph check app.ts" },
    { hl: [27, 32], tone: "err", mark: ["limit: 20", "limit: 1000"] },
    { out: [...overBudget("get_posts", 26, 20020, "reads", 250), { t: "" }, { t: "3 problems" }] },
    { pause: 1200 },
    {
      agent:
        "agent ▸ 20 posts × 1000 comments is over the read limit of 250. Showing 10 per post; the full thread can page in on demand.",
    },
    { hl: [] },
    { edit: 32, from: "limit: 1000 ", to: "limit: 10 " },
    { clear: true },
    { cmd: "$ aleph check app.ts" },
    {
      out: [
        { t: "✔ valid. The most each route and handler can use in one run:", k: "ok" },
        { t: "" },
        { t: TABLE_HEAD, k: "dim", table: true },
        { t: LIMIT_ROW, k: "dim", table: true },
        {
          t: "get_posts     220 88%     0  –     192 10%   769.6 KB 38%      0  –     0 –   2.38 MB 30%",
          hl: true,
          table: true,
        },
      ],
    },
  ] satisfies Step[],
};
