import { DevEngine, validate } from "@aleph-lang/core";
import { describe, expect, it } from "vitest";
import app from "../app.ts";

describe("todos", () => {
  it("is a valid app", () => {
    const result = validate(app.toJSON());
    expect(result.ok, JSON.stringify(result)).toBe(true);
  });

  it("adds, pages, and completes todos per user", () => {
    const engine = new DevEngine(app.build());
    const ada = { userId: "ada" };
    // Ids are time-ordered, so give each insert its own millisecond.
    ["one", "two", "three"].forEach((title, i) => {
      const response = engine.handle(
        { method: "POST", path: "/todos", caller: ada, body: { title } },
        1_000 + i,
      );
      expect(response.status).toBe(201);
    });
    const page = engine.handle({
      method: "GET",
      path: "/todos",
      caller: ada,
      query: { limit: "2" },
    });
    const body = page.body as {
      open: number;
      page: { items: { todoId: string; title: string }[]; cursor: string };
    };
    expect(body.open).toBe(3);
    expect(body.page.items.map((t) => t.title)).toEqual(["three", "two"]);
    const first = body.page.items[0];
    if (!first) throw new Error("expected a todo");
    engine.handle({ method: "POST", path: `/todos/${first.todoId}/done`, caller: ada });
    const after = engine.handle({ method: "GET", path: "/todos", caller: { userId: "bob" } });
    expect((after.body as { open: number }).open).toBe(0); // bob sees none of ada's
  });

  it("range queries skip todos with no due date, like SQL comparisons", () => {
    const engine = new DevEngine(app.build());
    const ada = { userId: "ada" };
    engine.handle({ method: "POST", path: "/todos", caller: ada, body: { title: "someday" } }, 1);
    engine.handle(
      { method: "POST", path: "/todos", caller: ada, body: { title: "soon", dueAt: 5_000 } },
      2,
    );
    engine.handle(
      { method: "POST", path: "/todos", caller: ada, body: { title: "later", dueAt: 90_000 } },
      3,
    );
    const due = engine.handle({
      method: "GET",
      path: "/todos/due",
      caller: ada,
      query: { before: "10000" },
    });
    expect((due.body as { title: string }[]).map((t) => t.title)).toEqual(["soon"]);
  });
});
