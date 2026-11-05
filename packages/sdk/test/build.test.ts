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
      /only be used inside a route handler/,
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
});
