/**
 * Order fulfillment as a durable workflow (a saga):
 * 1. charge the card
 * 2. reserve stock (if that fails, refund the charge)
 * 3. wait up to a day for a manager's approval (on timeout, refund and cancel)
 * 4. ship
 *
 * Each step is its own bounded transaction, with state carried between them. Routes start
 * instances, signal them, read their status, and cancel them.
 */
import { app, err, t } from "@aleph-lang/sdk";

const a = app("fulfillment");

const Order = a.entity("Order", {
  key: ["orderId"],
  fields: {
    orderId: t.string().min(1).max(32),
    customerId: t.string().max(64),
    amountCents: t.int().min(1).max(10_000_000),
    status: t.enum("new", "charged", "reserved", "shipped", "refunded").default("new"),
  },
});

const payments = a.integration("payments", {
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

const inventory = a.integration("inventory", {
  provider: "http",
  operations: {
    reserve: {
      mode: "call",
      input: { orderId: t.string().max(32) },
      output: { reservationId: t.string().max(64) },
    },
  },
});

const mail = a.integration("mail", {
  provider: "email",
  operations: {
    send: { mode: "effect", input: { to: t.string().max(64), subject: t.string().max(120) } },
  },
});

const Fulfill = a.workflow("fulfill", {
  input: { orderId: t.string().max(32) },
  state: { chargeId: t.string().max(64), reservationId: t.string().max(64) },
  signals: { approved: { by: t.string().max(64) } },
  maxSteps: 10,
  retries: 1,
  backoff: "1s",
});

Fulfill.step(
  "charge",
  (_, w) => {
    const order = Order.getOrFail({ orderId: w.input.orderId }, w.errors.gone);
    const charge = payments.call("charge", {
      orderId: order.orderId,
      amountCents: order.amountCents,
    });
    order.update({ status: "charged" });
    w.next("reserve", { state: { chargeId: charge.chargeId } });
  },
  { errors: { gone: err.notFound("The order was deleted") } },
);

Fulfill.step(
  "reserve",
  (_, w) => {
    const stock = inventory.call("reserve", { orderId: w.input.orderId });
    Order.update({ orderId: w.input.orderId }, { status: "reserved" });
    w.waitFor("approved", {
      timeout: "1d",
      onSignal: "ship",
      onTimeout: "refund",
      state: { reservationId: stock.reservationId },
    });
  },
  // A failed reservation is compensated: the charge is refunded.
  { onFail: "refund" },
);

Fulfill.step("ship", ($, w) => {
  const order = Order.get({ orderId: w.input.orderId });
  order.update({ status: "shipped" });
  mail.send("send", {
    to: order.customerId,
    subject: $.fmt`Shipped, approved by ${w.signals.approved.by}`,
  });
  w.complete({ shipped: true, approvedBy: w.signals.approved.by });
});

Fulfill.step("refund", ($, w) => {
  $.when(w.state.chargeId.isSet(), () => {
    payments.send("refund", { chargeId: w.state.chargeId.ifNull("") });
  });
  Order.update({ orderId: w.input.orderId }, { status: "refunded" });
  w.complete({ shipped: false, reason: w.error.ifNull("not approved in time") });
});

a.route("POST /orders")
  .auth("public")
  .body({
    orderId: t.string().min(1).max(32),
    customerId: t.string().max(64),
    amountCents: t.int().min(1).max(10_000_000),
  })
  .status(201)
  .example({
    name: "places an order and starts fulfillment",
    request: { body: { orderId: "o1", customerId: "ada", amountCents: 500 } },
    mocks: [
      { integration: "payments", operation: "charge", response: { chargeId: "ch_1" } },
      { integration: "inventory", operation: "reserve", response: { reservationId: "r_1" } },
    ],
    process: true,
    expect: { status: 201, body: { started: true }, runs: ["fulfill_charge", "fulfill_reserve"] },
  })
  .handle((_, { body }) => {
    Order.insert(body);
    return { started: Fulfill.start(body.orderId, { orderId: body.orderId }) };
  });

a.route("POST /orders/:orderId/approve")
  .auth("signedIn")
  .params({ orderId: t.string().min(1).max(32) })
  .handle(($, { params }) => {
    Fulfill.signal(params.orderId, "approved", { by: $.caller });
    return { signalled: true };
  });

a.route("GET /orders/:orderId/fulfillment")
  .auth("public")
  .params({ orderId: t.string().min(1).max(32) })
  .errors({ none: err.notFound("No fulfillment for this order") })
  .handle(($, { params, errors }) => {
    const run = Fulfill.get(params.orderId);
    $.ensure(run.status.isSet(), errors.none);
    return {
      status: run.status,
      step: run.step,
      chargeId: run.state.chargeId,
      result: run.result,
      error: run.error,
      steps: run.steps,
    };
  });

a.route("DELETE /orders/:orderId/fulfillment")
  .auth("signedIn")
  .params({ orderId: t.string().min(1).max(32) })
  .status(202)
  .handle((_, { params }) => {
    Fulfill.cancel(params.orderId);
    return { cancelling: true };
  });

export default a;
