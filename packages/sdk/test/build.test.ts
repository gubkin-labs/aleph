import { describe, expect, it } from "vitest";
import { AlephBuildError, app, type ErrorRef, err, t } from "../src/index.js";

function todoApp() {
  const a = app("todo");
  const Todo = a.entity("Todo", {
    key: ["ownerId", "todoId"],
    fields: {
      ownerId: t.string().max(64),
      todoId: t.id().generated(),
      title: t.string().min(1).max(200),
      done: t.bool().default(false),
    },
    indexes: { byDone: ["ownerId", "done"] },
  });
  const open = a.counter("openTodos", {
    of: Todo,
    groupBy: ["ownerId"],
    where: (row) => row.done.not(),
  });
  return { a, Todo, open };
}

describe("build", () => {
  it("records a route plan as manifest statements", () => {
    const { a, Todo } = todoApp();
    a.route("POST /todos")
      .body({ title: t.string().min(1).max(200) })
      .handle(($, { body }) => Todo.insert({ ownerId: $.caller, title: body.title }));

    const route = a.build().routes[0];
    expect(route?.id).toBe("post_todos");
    expect(route?.auth).toEqual({ kind: "signedIn" });
    expect(route?.plan).toEqual({
      body: [
        {
          kind: "let",
          var: 0,
          op: {
            op: "insert",
            entity: "Todo",
            values: {
              ownerId: { kind: "caller" },
              title: { kind: "input", source: "body", name: "title" },
            },
          },
        },
      ],
      output: { kind: "var", id: 0 },
    });
  });

  it("derives path params and records guards, branches, and counters", () => {
    const { a, Todo, open } = todoApp();
    a.route("POST /todos/:todoId/done")
      .errors({ notFound: err.notFound(), tooMany: err.conflict("too many open") })
      .handle(($, { params, errors }) => {
        const todo = Todo.getOrFail({ ownerId: $.caller, todoId: params.todoId }, errors.notFound);
        $.ensure(open.get({ ownerId: $.caller }).lt(100), errors.tooMany);
        $.if(todo.done)
          .then(() => $.fail(errors.tooMany))
          .else(() => {
            todo.update({ done: true });
          });
        return { title: todo.title };
      });

    const route = a.build().routes[0];
    expect(route?.input.params).toEqual({ todoId: { type: { kind: "string", min: 0, max: 128 } } });
    expect(route?.plan.body.map((s) => s.kind)).toEqual(["let", "ensure", "let", "ensure", "if"]);
    expect(a.build().counters["openTodos"]?.where).toEqual({
      var: 0,
      expr: { kind: "not", arg: { kind: "field", of: { kind: "var", id: 0 }, name: "done" } },
    });
  });

  it("records map and filter bodies as read-only blocks", () => {
    const { a, Todo } = todoApp();
    a.route("POST /todos/complete")
      .body({ todoIds: t.array(t.id(), 25) })
      .handle(($, { body }) => {
        const todos = Todo.getMany(body.todoIds.map((todoId) => ({ ownerId: $.caller, todoId })));
        const open = todos.filter((todo) => todo.exists().and(todo.done.not()));
        open.updateAll({ done: true });
        return { completed: open.length };
      });
    const ops = a.build().routes[0]?.plan.body.map((s) => (s.kind === "let" ? s.op.op : s.kind));
    expect(ops).toEqual(["map", "map", "filter", "forEach"]);
  });

  it("refuses writes inside map, with an actionable message", () => {
    const { a, Todo } = todoApp();
    expect(() =>
      a
        .route("POST /bad")
        .handle(() => Todo.query({ limit: 10 }).map((todo) => todo.update({ done: true }))),
    ).toThrow(/inside map\/filter/);
  });

  it("records a state-carrying loop, with shared values bound by $.let", () => {
    const { a, Todo } = todoApp();
    a.route("GET /streak").handle(($) => {
      const todos = Todo.query({ prefix: [$.caller], limit: 20 });
      const end = $.loop(
        { max: 20, state: { i: 0, streak: 0 }, while: (s) => s.i.lt(todos.length) },
        (s) => {
          const done = $.let(todos.slice(s.i, s.i.plus(1)).first().done.ifNull(false));
          return { i: s.i.plus(1), streak: $.select(done, s.streak.plus(1), 0) };
        },
      );
      return { streak: end.streak };
    });
    const body = a.build().routes[0]?.plan.body ?? [];
    const loop = body.find((s) => s.kind === "let" && s.op.op === "loop");
    expect(loop?.kind === "let" && loop.op.op === "loop" && loop.op).toMatchObject({
      max: 20,
      state: { i: { kind: "lit", value: 0 }, streak: { kind: "lit", value: 0 } },
      while: { kind: "binary", op: "lt" },
    });
    const inner = loop?.kind === "let" && loop.op.op === "loop" ? loop.op.body : [];
    expect(inner.map((s) => (s.kind === "let" ? s.op.op : s.kind))).toEqual(["eval"]);
  });

  it("refuses writes in a loop body, reads in its while, and a bad max", () => {
    const { a, Todo } = todoApp();
    expect(() =>
      a.route("POST /a").handle(($) =>
        $.loop({ max: 3, state: { n: 0 } }, (s) => {
          Todo.insert({ ownerId: $.caller, title: "x" });
          return { n: s.n.plus(1) };
        }),
      ),
    ).toThrow(/inside map\/filter\/loop/);
    expect(() =>
      a
        .route("POST /b")
        .handle(($) =>
          $.loop(
            { max: 3, state: { n: 0 }, while: () => Todo.query({ limit: 1 }).isEmpty() },
            (s) => ({ n: s.n.plus(1) }),
          ),
        ),
    ).toThrow(/while condition can only compute from the state/);
    expect(() =>
      a.route("POST /c").handle(($) => $.loop({ max: 0, state: { n: 0 } }, (s) => ({ n: s.n }))),
    ).toThrow(/max must be an integer from 1 to 1000/);
  });

  it("catches expressions used as plain JavaScript values", () => {
    const { a } = todoApp();
    expect(() => a.route("GET /bad").handle(($) => `${String($.now())}`)).toThrow(AlephBuildError);
  });

  it("reports undeclared errors at build time", () => {
    const { a } = todoApp();
    expect(() =>
      a
        .route("POST /x")
        .errors({ known: err.badRequest() })
        .handle(($, { errors }) => {
          $.fail(Reflect.get(errors, "unknown") as ErrorRef);
        }),
    ).toThrow(/has no error "unknown"/);
  });

  it("refuses data operations outside handlers", () => {
    const { Todo } = todoApp();
    expect(() => Todo.get({ ownerId: "a", todoId: "b" })).toThrow(
      /only be used inside a route or handler/,
    );
  });

  it("produces byte-identical JSON on every build", () => {
    const build = () => {
      const { a, Todo } = todoApp();
      a.route("GET /todos")
        .query({ limit: t.int().min(1).max(50).default(20) })
        .handle(($, { query }) =>
          Todo.query({
            index: "byDone",
            prefix: [$.caller, false],
            limit: query.limit,
            order: "desc",
          }),
        );
      return a.toJSON();
    };
    expect(build()).toBe(build());
    expect(build()).toMatch(/^\{\n {2}"app"/);
  });

  it("rejects floats and duplicate declarations", () => {
    const { a } = todoApp();
    expect(() => a.route("GET /f").handle(() => 1.5)).toThrow(/safe integer/);
    expect(() => a.entity("Todo", { key: ["id"], fields: { id: t.id() } })).toThrow(
      /declared twice/,
    );
    expect(() => app("x").route("FETCH /x")).toThrow(/must look like/);
  });

  it("records member auth, idempotency, pages, and upserts", () => {
    const a = app("org", { roles: ["owner", "member"], apiKeyScopes: ["read"] });
    const Doc = a.entity("Doc", {
      key: ["orgId", "docId"],
      fields: { orgId: t.string().max(64), docId: t.id().generated(), title: t.string().max(80) },
    });
    a.route("GET /orgs/:orgId/docs")
      .member("orgId", { roles: ["owner"], apiKeyScopes: ["read"] })
      .idempotent({ ttl: "15m", required: true })
      .query({ cursor: t.cursor() })
      .handle(($, { params, query }) => {
        Doc.upsert({ orgId: params.orgId, title: "x" });
        return {
          page: Doc.page({ prefix: [params.orgId], limit: 10, after: query.cursor }),
          role: $.callerRole(params.orgId),
        };
      });
    const m = a.build();
    const route = m.routes[0];
    expect(m.auth).toEqual({ roles: ["owner", "member"], apiKeyScopes: ["read"] });
    expect(route?.auth).toEqual({
      kind: "member",
      org: "orgId",
      roles: ["owner"],
      apiKeyScopes: ["read"],
    });
    expect(route?.idempotency).toEqual({ ttlMs: 900_000, required: true });
    const ops = route?.plan.body.map((s) => (s.kind === "let" ? s.op : undefined));
    expect(ops?.[0]?.op).toBe("upsert");
    expect(ops?.[1]).toMatchObject({
      op: "query",
      page: true,
      after: { kind: "input", name: "cursor" },
    });
    expect(() => a.route("GET /x").idempotent({ ttl: "soon" as "1s" })).toThrow(/invalid duration/);
  });

  it("refuses effects inside map and records handlers with source maps", () => {
    const a = app("fx");
    const Ping = a.event("ping", { payload: { n: t.int() } });
    const Note = a.entity("Note", { key: ["id"], fields: { id: t.id().generated() } });
    expect(() =>
      a.route("POST /x").handle(() => Note.query({ limit: 3 }).map(() => Ping.emit({ n: 1 }))),
    ).toThrow(/release effects/);
    a.onEvent("onPing", Ping, (_, { payload }) => {
      Note.insert({});
      return payload.n;
    });
    const m = a.build({ sourceMap: true });
    expect(m.handlers?.[0]).toMatchObject({
      id: "onPing",
      trigger: { kind: "event", event: "ping" },
    });
    expect(m.events).toEqual({ ping: { payload: { n: { type: { kind: "int" } } } } });
    // Without the option, builds stay free of machine-specific paths.
    expect(a.build().sourceMap).toBeUndefined();
  });
});
