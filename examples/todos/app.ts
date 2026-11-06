/** A todo app: signed-in users, CRUD, cursor paging, a range query on an index, and a counter. */
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

a.route("POST /todos")
  .id("addTodo")
  .body({ title: t.string().min(1).max(200), dueAt: t.timestamp().optional() })
  .handle(($, { body }) =>
    Todo.insert({ ownerId: $.caller, title: body.title.trim(), dueAt: body.dueAt }),
  );

a.route("GET /todos")
  .id("listTodos")
  .query({ limit: t.int().min(1).max(50).default(5), cursor: t.cursor() })
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

a.route("POST /todos/:todoId/done")
  .id("completeTodo")
  .params({ todoId: t.id() })
  .errors({ notFound: err.notFound("No such todo") })
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
