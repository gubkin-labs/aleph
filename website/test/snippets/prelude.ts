/**
 * Names the docs' code fragments refer to, with their real SDK types, so every fragment can be
 * type-checked against the actual API (see test/snippets.test.ts).
 */
import { app, err, t } from "@aleph-lang/sdk";

export const a = app("docs", {
  roles: ["owner", "admin", "member"],
  apiKeyScopes: ["members:read"],
  profile: { email: t.string().max(200), name: t.string().max(100) },
});
const orgId = t.string().max(64);

export const Org = a.entity("Org", {
  key: ["orgId"],
  fields: {
    orgId,
    name: t.string().max(80),
    plan: t.enum("free", "pro", "trial").default("free"),
    trialEndsAt: t.timestamp().optional(),
  },
  indexes: { byPlan: ["plan"] },
});
export const Task = a.entity("Task", {
  key: ["orgId", "taskId"],
  fields: {
    orgId,
    taskId: t.id().generated(),
    title: t.string().max(200),
    done: t.bool().default(false),
    status: t.enum("open", "blocked", "closed").default("open"),
    dueAt: t.timestamp().optional(),
  },
  indexes: { byDue: ["orgId", "done", "dueAt"], byStatus: ["orgId", "status"] },
});
export const Member = a.entity("Member", {
  key: ["orgId", "memberId"],
  fields: { orgId, memberId: t.id().generated(), email: t.string().max(200) },
  indexes: {
    byEmail: { fields: ["orgId", { name: "email", value: (m) => m.email.lower() }], unique: true },
  },
});
export const User = a.entity("User", {
  key: ["userId"],
  fields: { userId: t.string().max(64), name: t.string().max(100) },
});
export const Project = a.entity("Project", {
  key: ["orgId", "projectId"],
  fields: { orgId, projectId: t.id().generated(), name: t.string().max(120) },
});
export const Product = a.entity("Product", {
  key: ["sku"],
  fields: {
    sku: t.string().max(32),
    price: t.decimal(2),
    category: t.enum("books", "games", "tools"),
  },
});
export const Order = a.entity("Order", {
  key: ["orderId"],
  fields: {
    orderId: t.string().max(32),
    customerId: t.string().max(64),
    amountCents: t.int().min(1),
    totalCents: t.int().min(1).optional(),
    tz: t.timeZone().default("UTC"),
    placedAt: t.timestamp().defaultNow(),
  },
});
export const Account = a.entity("Account", {
  key: ["accountId"],
  fields: { accountId: t.id().generated(), balance: t.int().default(0) },
});
export const Post = a.entity("Post", {
  key: ["postId"],
  fields: { postId: t.id().generated(), title: t.string().max(100) },
});
export const OrderPaid = a.event("orderPaid", {
  payload: { orderId: t.id(), customerId: t.string().max(64) },
});
export const email = a.integration("email", {
  provider: "email",
  operations: {
    send: { mode: "effect", input: { to: t.string().max(200), subject: t.string().max(200) } },
  },
});
export const mail = email;
export const payments = a.integration("payments", {
  provider: "stripe",
  operations: {
    charge: {
      mode: "call",
      input: { orderId: t.string().max(32), amountCents: t.int() },
      output: { chargeId: t.string().max(64) },
    },
    refund: { mode: "effect", input: { chargeId: t.string().max(64) } },
  },
});
export const inventory = a.integration("inventory", {
  provider: "http",
  operations: {
    reserve: {
      mode: "call",
      input: { orderId: t.string().max(32) },
      output: { reservationId: t.string().max(64) },
    },
  },
});
export { err, t };
