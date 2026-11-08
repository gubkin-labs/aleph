import { app, t, verify } from "@aleph-lang/sdk";

const a = app("shop");

const Order = a.entity("Order", {
  key: ["customerId", "orderId"],
  fields: {
    customerId: t.string().max(64),
    orderId: t.id().generated(),
    item: t.string().max(64),
    status: t.enum("pending", "paid").default("pending"),
  },
});

a.route("POST /orders")
  .idempotent({ ttl: "24h" })
  .rateLimit({ per: "user", max: 10, window: "1m" })
  .body({ item: t.string().min(1).max(64) })
  .status(201)
  .handle(($, { body }) => Order.insert({ customerId: $.caller, item: body.item }));

a.webhook("POST /webhooks/stripe", {
  verify: verify.stripe("STRIPE_WEBHOOK_SECRET"),
  dedupe: { bodyField: "id", ttl: "3d" },
})
  .body({ id: t.string().max(64) })
  .handle(() => ({ received: true }));

export default a;
