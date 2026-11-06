/**
 * A team SaaS: org roles and API keys, case-insensitive unique emails, seat limits by plan,
 * idempotent invites, a last-owner guard, and a route exposed as an MCP tool.
 */
import { type App, app, err, t } from "@aleph/sdk";

const a: App<"owner" | "admin" | "member", "members:read"> = app("team", {
  roles: ["owner", "admin", "member"],
  apiKeyScopes: ["members:read"],
});
const orgId = t.string().min(1).max(64);

const Org = a.entity("Org", {
  key: ["orgId"],
  fields: { orgId, name: t.string().min(1).max(80), plan: t.enum("free", "pro").default("free") },
});

const Member = a.entity("Member", {
  key: ["orgId", "memberId"],
  fields: {
    orgId,
    memberId: t.id().generated(),
    email: t.string().min(3).max(200),
    role: t.enum("owner", "admin", "member").default("member"),
  },
  indexes: {
    // One membership per email per org, ignoring case.
    byEmail: {
      fields: ["orgId", { name: "emailLower", value: (m) => m.email.lower() }],
      unique: true,
    },
  },
});

const seats = a.counter("seats", { of: Member, groupBy: ["orgId"] });
const owners = a.counter("owners", {
  of: Member,
  groupBy: ["orgId"],
  where: (m) => m.role.eq("owner"),
});

a.route("PUT /orgs/:orgId")
  .id("saveOrg")
  .params({ orgId })
  .member("orgId", { roles: ["owner"] })
  .body({ name: t.string().min(1).max(80), plan: t.enum("free", "pro") })
  .handle((_, { params, body }) =>
    Org.upsert({ orgId: params.orgId, name: body.name, plan: body.plan }),
  );

a.route("GET /orgs/:orgId/members")
  .id("listMembers")
  .params({ orgId })
  .member("orgId", { apiKeyScopes: ["members:read"] })
  .query({ cursor: t.cursor() })
  .handle((_, { params, query }) => ({
    seats: seats.get({ orgId: params.orgId }),
    page: Member.page({ prefix: [params.orgId], limit: 20, after: query.cursor }),
  }));

a.route("POST /orgs/:orgId/members")
  .id("invite")
  .params({ orgId })
  .member("orgId", { roles: ["owner", "admin"] })
  .idempotent({ ttl: "24h" }) // a retried invite never adds the member twice
  .body({
    email: t.string().min(3).max(200),
    role: t.enum("owner", "admin", "member").default("member"),
  })
  .errors({
    seatLimit: err.forbidden("The free plan has 3 seats"),
    noOrg: err.notFound("Create the org first"),
  })
  .handle(($, { params, body, errors }) => {
    const org = Org.getOrFail({ orgId: params.orgId }, errors.noOrg);
    const maxSeats = $.match(org.plan, { free: 3, pro: 500 });
    $.ensure(seats.get({ orgId: params.orgId }).lt(maxSeats), errors.seatLimit);
    // A duplicate email fails with 409 unique_violation from the index.
    return Member.insert({ orgId: params.orgId, email: body.email.trim(), role: body.role });
  });

a.route("GET /orgs/:orgId/members/by-email/:email")
  .id("findMember")
  .params({ orgId, email: t.string().min(3).max(200) })
  .member("orgId", { apiKeyScopes: ["members:read"] }) // members, or an API key with the scope
  .describe("Find a member by email")
  .mcp("find_member") // also an MCP tool, with the same auth and limits
  .errors({ notFound: err.notFound() })
  .handle(($, { params, errors }) => {
    const member = Member.findBy("byEmail", [params.orgId, params.email.lower()]);
    $.ensure(member.exists(), errors.notFound);
    return member;
  });

a.route("DELETE /orgs/:orgId/members/:memberId")
  .id("removeMember")
  .params({ orgId, memberId: t.id() })
  .member("orgId", { roles: ["owner"] })
  .errors({ notFound: err.notFound(), lastOwner: err.conflict("Can't remove the last owner") })
  .handle(($, { params, errors }) => {
    const member = Member.getOrFail(params, errors.notFound);
    $.ensure(
      $.any(member.role.neq("owner"), owners.get({ orgId: params.orgId }).gt(1)),
      errors.lastOwner,
    );
    member.delete();
    return { removed: true };
  });

export default a;
