import { DevEngine, validate } from "@aleph/core";
import { describe, expect, it } from "vitest";
import app from "../app.ts";

const ada = { userId: "ada" };

describe("shop", () => {
  it("is a valid app", () => {
    const result = validate(app.toJSON());
    expect(result.ok, JSON.stringify(result)).toBe(true);
  });

  it("charges orders asynchronously, retries, and dead-letters", () => {
    const engine = new DevEngine(app.build());
    engine.mock("payments", "charge", { chargeId: "ch_1" });
    const order = (key: string, now = 1_000) =>
      engine.handle(
        {
          method: "POST",
          path: "/orders",
          caller: ada,
          body: { totalCents: 2_500 },
          idempotencyKey: key,
        },
        now,
      );

    expect(order("k1").status).toBe(201);
    expect(order("k1").replayed).toBe(true); // no second order
    const report = engine.process(1_000);
    expect(report.runs.map((r) => r.handler).sort()).toEqual([
      "chargeOrder",
      "expireStale",
      "receipt",
    ]);
    expect(engine.takeDeliveries()).toMatchObject([{ kind: "send", target: "email" }]);
    const list = engine.handle({ method: "GET", path: "/orders", caller: ada }, 1_001);
    expect(list.body).toMatchObject({
      revenueCents: 2_500,
      page: { items: [{ status: "paid", chargeId: "ch_1" }] },
    });

    engine.mock("payments", "charge", { error: "down" });
    order("k2", 1_500); // a later id, so it lists first
    engine.process(2_000); // attempt 1 fails
    engine.process(4_000); // after 2s, attempt 2 fails
    const last = engine.process(8_000); // after 4s, attempt 3 fails, then dead-letter
    expect(last).toMatchObject({ deadLettered: 1 });
    const after = engine.handle({ method: "GET", path: "/orders", caller: ada }, 8_001);
    expect((after.body as { page: { items: { status: string }[] } }).page.items[0]).toMatchObject({
      status: "failed",
    });
  });
});
