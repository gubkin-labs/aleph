/**
 * The homepage playground. Each example has a few options; each option has the code it stands
 * for and the sequence the visual plays. Every number and message comes from running `aleph check`
 * or `aleph dev` on that exact code (in a small blog app with Post and Comment entities).
 */

export type Tone = "ok" | "err" | "dim";

export type Item =
  | { box: string; sub?: string; tone?: Tone }
  | { op: string }
  | { meter: string; value: number; max: number }
  | { result: string; tone: Tone };

export interface Option {
  label: string;
  code: string[];
  /** 1-based lines to mark as changed. */
  hl?: number[];
  seq: Item[];
}

export interface Example {
  name: string;
  description: string;
  control: string;
  options: Option[];
}

export interface Category {
  name: string;
  examples: Example[];
}

const queryCode = (limit: number) => [
  'a.route("GET /posts")',
  `  .handle(() => Post.query({ limit: ${limit} }));`,
];

const pageCode = [
  'a.route("GET /posts")',
  "  .query({ cursor: t.cursor() })",
  "  .handle((_, { query }) =>",
  "    Post.page({ limit: 20, after: query.cursor }));",
];

const mapCode = (k: number) => [
  'a.route("GET /posts")',
  "  .handle(() =>",
  "    Post.query({ limit: 20 }).map((post) => ({",
  "      post,",
  `      comments: Comment.query({ prefix: [post.postId], limit: ${k} }),`,
  "    })));",
];

const filterCode = (n: number) => [
  'a.route("POST /posts")',
  `  .body({ tags: t.array(t.string().max(32), ${n}) })`,
  "  .handle((_, { body }) => ({",
  "    unique: body.tags.filter((tag) =>",
  "      body.tags.filter((other) => other.eq(tag)).length.eq(1)),",
  "  }));",
];

const txCode = [
  'a.route("POST /posts/:postId/comments")',
  "  .params({ postId: t.string().max(26) })",
  "  .body({ text: t.string().min(1).max(500) })",
  '  .errors({ noPost: err.notFound("No such post") })',
  "  .status(201)",
  "  .handle(($, { params, body, errors }) => {",
  "    const comment = Comment.insert({",
  "      postId: params.postId,",
  "      authorId: $.caller,",
  "      text: body.text,",
  "    });",
  '    email.send("send", { to: "author@example.com", subject: "New comment" });',
  "    const post = Post.getOrFail({ postId: params.postId }, errors.noPost);",
  "    post.update({ comments: post.comments.plus(1) });",
  "    return comment;",
  "  });",
];

const idemCode = [
  'a.route("POST /posts")',
  '  .idempotent({ ttl: "1h" })',
  "  .body({ title: t.string().min(1).max(200) })",
  "  .status(201)",
  "  .handle((_, { body }) => Post.insert({ title: body.title }));",
];

const rateCode = [
  'a.route("POST /posts/:postId/comments")',
  '  .rateLimit({ per: "user", max: 3, window: "1m" })',
  "  .params({ postId: t.string().max(26) })",
  "  .body({ text: t.string().min(1).max(500) })",
  "  .handle(($, { params, body }) =>",
  "    Comment.insert({",
  "      postId: params.postId,",
  "      authorId: $.caller,",
  "      text: body.text,",
  "    }));",
];

const entity = (extra: string | null, text = "500") => [
  'const Comment = a.entity("Comment", {',
  '  key: ["postId", "commentId"],',
  "  fields: {",
  "    postId: t.string().max(26),",
  "    commentId: t.id().generated(),",
  `    text: t.string().min(1).max(${text}),`,
  ...(extra ? [extra] : []),
  "  },",
  "});",
];

const requests = (n: number): Item[] =>
  Array.from({ length: n }, (_, i) =>
    i < 3 ? { box: "POST", sub: "201", tone: "ok" } : { box: "POST", sub: "429", tone: "err" },
  );

export const PLAYGROUND: Category[] = [
  {
    name: "Cost",
    examples: [
      {
        name: "Post.query",
        description: "Read a range of rows. Every query has a limit, so every read has a bound.",
        control: "limit",
        options: [10, 100, 1000].map((limit) => ({
          label: String(limit),
          code: queryCode(limit),
          hl: [2],
          seq:
            limit <= 100
              ? [
                  { box: "GET /posts" },
                  { op: "→" },
                  { box: `≤ ${limit} rows` },
                  { meter: "reads", value: limit, max: 250 },
                  { result: `✔ ${limit} reads (${(limit / 250) * 100}%)`, tone: "ok" },
                ]
              : [
                  { box: "GET /posts" },
                  { op: "→" },
                  { box: "≤ 1000 rows", tone: "err" },
                  { meter: "reads", value: 1000, max: 250 },
                  { result: "✖ up to 1000 reads, over the limit of 250", tone: "err" },
                ],
        })),
      },
      {
        name: "Post.page",
        description:
          "Page through any amount of data. Each request reads one page, however big the table.",
        control: "posts stored",
        options: [
          ["100", 5],
          ["10,000", 500],
          ["1,000,000", 50_000],
        ].map(([label, n]) => ({
          label: String(label),
          code: pageCode,
          hl: [2, 4],
          seq: [
            { box: "GET /posts", sub: "20 + cursor" },
            { op: "→" },
            { box: "GET ?cursor=…", sub: "20 + cursor" },
            { op: "→" },
            { box: "…", sub: `${Number(n).toLocaleString("en-US")} requests` },
            { result: "✔ every request: 20 reads (8%)", tone: "ok" },
          ],
        })),
      },
      {
        name: "list.map",
        description: "Loops multiply bounds. 20 posts with k comments each is 20 + 20 × k reads.",
        control: "comments per post",
        options: [5, 10, 50].map((k) => {
          const reads = 20 + 20 * k;
          const ok = reads <= 250;
          return {
            label: String(k),
            code: mapCode(k),
            hl: [5],
            seq: [
              { box: "20 posts" },
              { op: "×" },
              { box: `${k} comments`, ...(ok ? {} : { tone: "err" as const }) },
              { meter: "reads", value: reads, max: 250 },
              ok
                ? { result: `✔ ${reads} reads (${Math.round((reads / 250) * 100)}%)`, tone: "ok" }
                : { result: `✖ up to ${reads} reads, over the limit of 250`, tone: "err" },
            ],
          };
        }),
      },
      {
        name: "list.filter",
        description:
          "There's no while loop. Every loop runs over a list with a maximum size, so CPU is bounded too.",
        control: "max tags",
        options: [
          [10, 485],
          [20, 1765],
          [100, 40_805],
        ].map(([n, ops]) => {
          const ok = Number(ops) <= 2000;
          return {
            label: String(n),
            code: filterCode(Number(n)),
            hl: [2],
            seq: [
              { box: `${n} tags` },
              { op: "×" },
              { box: `${n} tags`, ...(ok ? {} : { tone: "err" as const }) },
              { meter: "ops", value: Number(ops), max: 2000 },
              ok
                ? {
                    result: `✔ ${ops} ops (${Math.round((Number(ops) / 2000) * 100)}%)`,
                    tone: "ok",
                  }
                : { result: "✖ up to 40805 ops, over the limit of 2000", tone: "err" },
            ],
          };
        }),
      },
    ],
  },
  {
    name: "Correctness",
    examples: [
      {
        name: "transactions",
        description:
          "Every request is one transaction. Effects like email go out only after it commits.",
        control: "the post",
        options: [
          {
            label: "exists",
            code: txCode,
            hl: [12],
            seq: [
              { box: "insert comment" },
              { box: "queue email" },
              { box: "update post" },
              { op: "→" },
              { result: "✔ 201, committed", tone: "ok" },
              { box: "email sent", sub: "once" },
            ],
          },
          {
            label: "was deleted",
            code: txCode,
            hl: [13],
            seq: [
              { box: "insert comment" },
              { box: "queue email" },
              { box: "post not found", tone: "err" },
              { op: "→" },
              { result: "✖ 404, rolled back", tone: "err" },
              { box: "no comment, no email", tone: "dim" },
            ],
          },
        ],
      },
      {
        name: ".idempotent",
        description:
          "Retried requests are safe. A repeated Idempotency-Key replays the stored response.",
        control: "second request",
        options: [
          {
            label: "same key",
            code: idemCode,
            hl: [2],
            seq: [
              { box: "key: k1", sub: "201 created", tone: "ok" },
              { op: "→" },
              { box: "key: k1", sub: "201 replayed", tone: "ok" },
              { result: "✔ one post, 0 writes on the retry", tone: "ok" },
            ],
          },
          {
            label: "new key",
            code: idemCode,
            hl: [2],
            seq: [
              { box: "key: k1", sub: "201 created", tone: "ok" },
              { op: "→" },
              { box: "key: k2", sub: "201 created", tone: "ok" },
              { result: "✔ two posts: different keys", tone: "ok" },
            ],
          },
        ],
      },
      {
        name: ".rateLimit",
        description: "Limits are declared, not coded. Over the limit, callers get a 429.",
        control: "requests in a minute",
        options: [3, 4, 6].map((n) => ({
          label: String(n),
          code: rateCode,
          hl: [2],
          seq: [
            ...requests(n),
            n <= 3
              ? { result: "✔ all served", tone: "ok" }
              : { result: `✖ ${n - 3} rate limited`, tone: "err" },
          ],
        })),
      },
    ],
  },
  {
    name: "Schema",
    examples: [
      {
        name: "check --against",
        description:
          "Every change is compared with what's deployed. Breaking ones are blocked before they ship.",
        control: "change",
        options: [
          {
            label: "add a field",
            code: entity("    likes: t.int().min(0).default(0),"),
            hl: [7],
            seq: [
              { box: "deployed.json" },
              { op: "→" },
              { box: "new field likes" },
              { result: "✔ deployable", tone: "ok" },
            ],
          },
          {
            label: "widen",
            code: entity(null, "2000"),
            hl: [6],
            seq: [
              { box: "deployed.json" },
              { op: "→" },
              { box: "length range widened" },
              { result: "✔ deployable", tone: "ok" },
            ],
          },
          {
            label: "narrow",
            code: entity(null, "200"),
            hl: [6],
            seq: [
              { box: "deployed.json" },
              { op: "→" },
              { box: "1-500 narrowed to 1-200", tone: "err" },
              { result: "✖ existing values may not fit", tone: "err" },
            ],
          },
          {
            label: "required field",
            code: entity("    pinned: t.bool(),"),
            hl: [7],
            seq: [
              { box: "deployed.json" },
              { op: "→" },
              { box: "new required field", tone: "err" },
              { result: "✖ no value in existing rows", tone: "err" },
            ],
          },
        ],
      },
    ],
  },
];
