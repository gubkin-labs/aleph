/**
 * A shop: orders are charged by a queue job that calls a payment integration (journaled, so a
 * retry never charges twice), with retries, a dead-letter handler, an event that sends a receipt
 * email, a running revenue sum, and an hourly cron that expires stale pending orders.
 */
import { type App, app, err, t } from "@aleph/sdk";

const a: App<never, never> = app("shop");

const Order = a.entity("Order", {
  key: ["customerId", "orderId"],
  fields: {
    customerId: t.string().max(64),
    orderId: t.id().generated(),
    totalCents: t.int().min(1).max(10_000_000),
    status: t.enum("pending", "paid", "failed").default("pending"),
    chargeId: t.string().max(100).optional(),
    createdAt: t.timestamp().defaultNow(),
  },
  indexes: { pendingByAge: { fields: ["createdAt"], where: (o) => o.status.eq("pending") } },
  invariants: { positiveTotal: (o) => o.totalCents.gt(0) },
});

const revenue = a.counter("revenueCents", {
  of: Order,
  groupBy: [],
  where: (o) => o.status.eq("paid"),
  sum: (o) => o.totalCents, // a running sum, maintained on every write
});

const Charge = a.queue("charge", {
  payload: { customerId: t.string().max(64), orderId: t.id() },
  retries: 2,
  backoff: "2s",
});

const OrderPaid = a.event("orderPaid", {
  payload: { customerId: t.string().max(64), orderId: t.id() },
});

const payments = a.integration("payments", {
  provider: "stripe",
  config: { secret: "STRIPE_SECRET_KEY" },
  operations: {
    charge: {
      mode: "call", // returns data; journaled, so a retried attempt never charges twice
      input: { orderId: t.id(), amountCents: t.int().min(1) },
      output: { chargeId: t.string().max(100) },
    },
  },
});

const email = a.integration("email", {
  provider: "email",
  operations: {
    send: { mode: "effect", input: { to: t.string().max(200), subject: t.string().max(200) } },
  },
});

a.route("POST /orders")
  .id("placeOrder")
  .rateLimit({ per: "user", max: 10, window: "1m" })
  .idempotent({ ttl: "1h", required: true })
  .body({ totalCents: t.int().min(1).max(10_000_000) })
  .handle(($, { body }) => {
    const order = Order.insert({ customerId: $.caller, totalCents: body.totalCents });
    Charge.enqueue({ customerId: $.caller, orderId: order.orderId }); // released after commit
    return order;
  });

a.route("GET /orders")
  .id("myOrders")
  .query({ cursor: t.cursor() })
  .handle(($, { query }) => ({
    revenueCents: revenue.get({}),
    page: Order.page({ prefix: [$.caller], limit: 20, after: query.cursor, order: "desc" }),
  }));

a.onQueue(
  "chargeOrder",
  Charge,
  (_, { payload, errors }) => {
    const order = Order.getOrFail(payload, errors.gone);
    const charge = payments.call("charge", {
      orderId: order.orderId,
      amountCents: order.totalCents,
    });
    order.update({ status: "paid", chargeId: charge.chargeId });
    OrderPaid.emit(payload);
  },
  { errors: { gone: err.notFound("Order was deleted") } },
);

a.onDeadLetter("chargeFailed", Charge, (_, { payload }) => {
  Order.update({ customerId: payload.customerId, orderId: payload.orderId }, { status: "failed" });
});

a.onEvent("receipt", OrderPaid, ($, { payload }) => {
  email.send("send", {
    to: $.fmt`${payload.customerId}@example.com`,
    subject: "Thanks for your order",
  });
});

a.every(
  "expireStale",
  "1h",
  ($, { payload }) => {
    $.forEach(payload.batch, (order) => {
      $.when(order.createdAt.lt(payload.firedAt.minus(86_400_000)), () => {
        order.update({ status: "failed" });
      });
    });
  },
  { scan: { entity: Order, index: "pendingByAge", batch: 40 } },
);

export default a;
