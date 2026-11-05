/**
 * Type-level tests: the SDK should reject mistakes at compile time, before the core sees them.
 * `tsc --noEmit` (pnpm typecheck) fails if any `@ts-expect-error` stops being an error.
 */
import { describe, expectTypeOf, it } from "vitest";
import { app, type Expr, err, type List, type Row, t } from "../src/index.js";

const a = app("types");
const User = a.entity("User", {
  key: ["userId"],
  fields: {
    userId: t.id().generated(),
    email: t.string().max(200),
    role: t.enum("admin", "member").default("member"),
    nickname: t.string().max(30).optional(),
  },
});

describe("types", () => {
  it("types row fields from their declarations", () => {
    a.route("GET /users/:userId").handle((_, { params }) => {
      const user = User.get({ userId: params.userId });
      expectTypeOf(user).toExtend<Row<typeof User.fields>>();
      expectTypeOf(user.role).toEqualTypeOf<Expr<"admin" | "member">>();
      expectTypeOf(user.nickname).toEqualTypeOf<Expr<string | null>>();
      expectTypeOf(params.userId).toEqualTypeOf<Expr<string>>();
      return user;
    });
  });

  it("requires exactly the fields an insert must provide", () => {
    a.route("POST /users").handle(() => {
      User.insert({ email: "a@b.c" });
      // @ts-expect-error: email is required (no default, not optional)
      User.insert({ role: "admin" });
      // @ts-expect-error: "owner" isn't a valid role
      User.insert({ email: "a@b.c", role: "owner" });
      return null;
    });
  });

  it("types inputs, lists, and errors", () => {
    a.route("POST /invite")
      .body({ emails: t.array(t.string().max(200), 10), max: t.int().max(5) })
      .errors({ full: err.conflict() })
      .handle(($, { body, errors }) => {
        expectTypeOf(body.emails).toEqualTypeOf<List<Expr<string>>>();
        expectTypeOf(body.max.plus(1)).toEqualTypeOf<Expr<number>>();
        $.ensure(body.emails.length.lte(body.max), errors.full);
        // Never called: these lines only need to fail type checking.
        const _typeErrors = () => {
          // @ts-expect-error: only declared errors exist
          $.fail(errors.missing);
          // @ts-expect-error: string expressions have no arithmetic
          body.emails.map((e) => e.plus(1));
        };
        return null;
      });
  });

  it("rejects strings without a max at the type level", () => {
    a.entity("Bad", {
      key: ["id"],
      // @ts-expect-error: t.string() needs .max(n) to be a field
      fields: { id: t.id(), text: t.string() },
    });
  });

  it("types roles against the app and org params against the path", () => {
    const b = app("typed", { roles: ["owner", "member"] });
    b.route("GET /orgs/:orgId").member("orgId", { roles: ["owner"] });
    const _typeErrors = () => {
      // @ts-expect-error: "admin" isn't a declared role
      b.route("GET /orgs/:orgId/a").member("orgId", { roles: ["admin"] });
      // @ts-expect-error: the path has no :teamId param
      b.route("GET /orgs/:orgId/b").member("teamId");
    };
  });
});
