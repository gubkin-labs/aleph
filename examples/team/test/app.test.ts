import { DevEngine, validate } from "@aleph/core";
import { describe, expect, it } from "vitest";
import app from "../app.ts";

const owner = { userId: "ada", orgs: { acme: "owner" } };

describe("team", () => {
  it("is a valid app", () => {
    const result = validate(app.toJSON());
    expect(result.ok, JSON.stringify(result)).toBe(true);
  });

  it("enforces roles, unique emails, seat limits, and idempotent invites", () => {
    const engine = new DevEngine(app.build());
    const invite = (email: string, key?: string, caller: object = owner) =>
      engine.handle({
        method: "POST",
        path: "/orgs/acme/members",
        caller,
        body: { email },
        ...(key ? { idempotencyKey: key } : {}),
      });

    expect(invite("a@acme.dev").status).toBe(404); // no org yet
    engine.handle({
      method: "PUT",
      path: "/orgs/acme",
      caller: owner,
      body: { name: "Acme", plan: "free" },
    });
    expect(invite("a@acme.dev", "k1").status).toBe(200);
    expect(invite("a@acme.dev", "k1").replayed).toBe(true);
    expect(invite("A@ACME.DEV").body).toMatchObject({ error: "unique_violation" });
    expect(invite("x@acme.dev", undefined, { userId: "cy", orgs: { acme: "member" } }).status).toBe(
      403,
    );
    expect(invite("b@acme.dev").status).toBe(200);
    expect(invite("c@acme.dev").status).toBe(200);
    expect(invite("d@acme.dev").body).toMatchObject({ error: "seatLimit" });

    const key = { apiKey: { orgId: "acme", scopes: ["members:read"] } };
    const found = engine.handle({
      method: "GET",
      path: "/orgs/acme/members/by-email/B@acme.dev",
      caller: key,
    });
    expect(found.body).toMatchObject({ email: "b@acme.dev" });
  });
});
