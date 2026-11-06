/**
 * A todo app: signed-in users, CRUD, cursor paging, a range query on an index, a counter, a
 * "due today" view in the user's time zone, and worked examples (`aleph test`).
 */
import { type App, app, err, t } from "@aleph/sdk";

const a: App<never, never> = app("todos");

const Todo = a.entity("Todo", {
  key: ["ownerId", "todoId"], // a user's todos are one contiguous range
  fields: {
    ownerId: t.string().max(64),
    todoId: t.id().generated(),
    title: t.string().min(1).max(200),
    done: t.bool().default(false),
    dueAt: t.timestamp().optional(),
  },
  indexes: { byDue: ["ownerId", "done", "dueAt"] },
});

const openTodos = a.counter("openTodos", {
  of: Todo,
  groupBy: ["ownerId"],
  where: (todo) => todo.done.not(),
});

const ada = { userId: "ada" };

a.route("POST /todos")
  .id("addTodo")
  .body({ title: t.string().min(1).max(200), dueAt: t.timestamp().optional() })
  .status(201)
  .example({
    name: "adds a todo, trimmed",
    as: ada,
    request: { body: { title: "  Buy milk  " } },
    expect: { status: 201, body: { title: "Buy milk", done: false, ownerId: "ada" } },
  })
  .handle(($, { body }) =>
    Todo.insert({ ownerId: $.caller, title: body.title.trim(), dueAt: body.dueAt }),
  );

a.route("GET /todos")
  .id("listTodos")
  .query({ limit: t.int().min(1).max(50).default(5), cursor: t.cursor() })
  .example({
    name: "lists newest first with the open count",
    as: ada,
    given: {
      Todo: [
        { ownerId: "ada", todoId: "01J00000000000000000000001", title: "Old", done: true },
        { ownerId: "ada", todoId: "01J00000000000000000000002", title: "New" },
        { ownerId: "bob", todoId: "01J00000000000000000000003", title: "Not mine" },
      ],
    },
    expect: {
      body: { open: 1, page: { items: [{ title: "New" }, { title: "Old" }], cursor: null } },
    },
  })
  .handle(($, { query }) => ({
    open: openTodos.get({ ownerId: $.caller }),
    page: Todo.page({ prefix: [$.caller], limit: query.limit, after: query.cursor, order: "desc" }),
  }));

a.route("GET /todos/due")
  .id("dueBefore")
  .query({ before: t.timestamp() })
  .handle(($, { query }) =>
    Todo.query({
      index: "byDue",
      prefix: [$.caller, false],
      range: { lt: query.before },
      limit: 50,
    }),
  );

a.route("GET /todos/today")
  .id("dueToday")
  .describe("Open todos due before the end of today, in the caller's time zone")
  .query({ tz: t.timeZone().default("UTC") })
  .example({
    name: "uses the caller's calendar day",
    as: ada,
    given: {
      Todo: [
        // 2023-11-14 23:30 UTC is already the 15th in Tokyo.
        {
          ownerId: "ada",
          todoId: "01J00000000000000000000001",
          title: "Tonight",
          dueAt: 1_700_004_600_000,
        },
        {
          ownerId: "ada",
          todoId: "01J00000000000000000000002",
          title: "Next week",
          dueAt: 1_700_600_000_000,
        },
      ],
    },
    request: { query: { tz: "Asia/Tokyo" } },
    expect: { body: { day: "2023-11-15", todos: [{ title: "Tonight" }] } },
  })
  .handle(($, { query }) => {
    const endOfDay = $.now().startOf("day", query.tz).add(1, "day", query.tz);
    return {
      day: $.now().formatTime("%Y-%m-%d", query.tz),
      todos: Todo.query({
        index: "byDue",
        prefix: [$.caller, false],
        range: { lt: endOfDay },
        limit: 50,
      }),
    };
  });

a.route("POST /todos/:todoId/done")
  .id("completeTodo")
  .params({ todoId: t.id() })
  .errors({ notFound: err.notFound("No such todo") })
  .example({
    name: "a missing todo is a 404",
    as: ada,
    request: { params: { todoId: "01J00000000000000000000009" } },
    expect: { status: 404, body: { error: "notFound" } },
  })
  .handle(($, { params, errors }) =>
    Todo.getOrFail({ ownerId: $.caller, todoId: params.todoId }, errors.notFound).update({
      done: true,
    }),
  );

a.route("DELETE /todos/:todoId")
  .id("deleteTodo")
  .params({ todoId: t.id() })
  .handle(($, { params }) => {
    Todo.delete({ ownerId: $.caller, todoId: params.todoId });
    return { deleted: true };
  });

export default a;
