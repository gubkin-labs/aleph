import { app, t } from "@aleph-lang/sdk";

const a = app("shop", { profile: { email: t.string().max(200) } });

const Order = a.entity("Order", {
  key: ["customerId", "orderId"],
  fields: {
    customerId: t.string().max(64),
    orderId: t.id().generated(),
    item: t.string().max(64),
  },
});

const email = a.integration("email", {
  provider: "email",
  operations: {
    send: { mode: "effect", input: { to: t.string().max(200), subject: t.string().max(200) } },
  },
});

a.route("POST /orders")
  .body({ item: t.string().max(64) })
  .status(201)
  .handle(($, { body }) => {
    const order = Order.insert({ customerId: $.caller, item: body.item });
    email.send("send", {
      to: a.caller.profile.email.ifNull("noreply@example.com"),
      subject: "Your receipt",
    });
    return order;
  });

export default a;
