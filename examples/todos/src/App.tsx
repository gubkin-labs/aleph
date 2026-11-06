import {
  type CallResult,
  Inspector,
  ResultView,
  useAleph,
  useQuery,
} from "@aleph-examples/inspector/react";
import { useState } from "react";

interface Todo {
  todoId: string;
  title: string;
  done: boolean;
  dueAt: number | null;
}

interface TodoList {
  open: number;
  page: { items: Todo[]; cursor: string | null };
}

const DAY = 86_400_000;

export function App() {
  const { call } = useAleph();
  const [title, setTitle] = useState("");
  const [dueInDays, setDueInDays] = useState("");
  const [cursor, setCursor] = useState<string | undefined>();
  const [last, setLast] = useState<CallResult>();
  const list = useQuery<TodoList>("/todos", { cursor });
  // Fixed once: a new timestamp on every render would refetch forever.
  const [dueBefore] = useState(() => Date.now() + 2 * DAY);
  const due = useQuery<Todo[]>("/todos/due", { before: dueBefore });

  async function add() {
    const body = { title, ...(dueInDays ? { dueAt: Date.now() + Number(dueInDays) * DAY } : {}) };
    setLast(await call("POST", "/todos", { body }));
    setTitle("");
  }

  return (
    <div className="al-layout">
      <main className="al-main">
        <h1>Todos</h1>
        <p>
          Each user's todos are one key range, so listing is a prefix scan. A counter keeps the open
          count without counting rows. Switch users in the inspector.
        </p>

        <section className="al-card">
          <h3>Add a todo</h3>
          <div className="al-row">
            <input
              aria-label="Title"
              placeholder="Title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
            />
            <input
              aria-label="Due in days"
              placeholder="due in days (optional)"
              value={dueInDays}
              onChange={(e) => setDueInDays(e.target.value)}
            />
            <button type="button" onClick={add}>
              Add
            </button>
          </div>
          <ResultView result={last} />
        </section>

        <section className="al-card">
          <h3>
            My todos <span className="al-dim">({list?.body.open ?? 0} open, from a counter)</span>
          </h3>
          {list && !list.ok ? <ResultView result={list} /> : null}
          <ul className="al-list">
            {list?.ok
              ? list.body.page.items.map((todo) => (
                  <li key={todo.todoId}>
                    <input
                      type="checkbox"
                      aria-label={`Complete ${todo.title}`}
                      checked={todo.done}
                      disabled={todo.done}
                      onChange={async () =>
                        setLast(await call("POST", `/todos/${todo.todoId}/done`))
                      }
                    />
                    <span style={{ textDecoration: todo.done ? "line-through" : "none", flex: 1 }}>
                      {todo.title}
                    </span>
                    {todo.dueAt ? (
                      <span className="al-dim">
                        due {new Date(todo.dueAt).toLocaleDateString()}
                      </span>
                    ) : null}
                    <button
                      type="button"
                      onClick={async () => setLast(await call("DELETE", `/todos/${todo.todoId}`))}
                    >
                      Delete
                    </button>
                  </li>
                ))
              : null}
          </ul>
          <div className="al-row">
            <button type="button" disabled={!cursor} onClick={() => setCursor(undefined)}>
              First page
            </button>
            <button
              type="button"
              disabled={!list?.ok || !list.body.page.cursor}
              onClick={() => setCursor(list?.body.page.cursor ?? undefined)}
            >
              Next page (cursor)
            </button>
          </div>
        </section>

        <section className="al-card">
          <h3>Due in the next 2 days</h3>
          <p className="al-dim">An index on (owner, done, dueAt), queried with a range on dueAt.</p>
          <ul className="al-list">
            {due?.ok ? due.body.map((todo) => <li key={todo.todoId}>{todo.title}</li>) : null}
          </ul>
        </section>
      </main>
      <Inspector />
    </div>
  );
}
