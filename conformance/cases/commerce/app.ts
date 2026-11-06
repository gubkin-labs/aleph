/**
 * A small shop that exercises language v0.3. It covers:
 * - **data:** object and list fields, exact decimals, time zones and calendar math, versioned
 *   products, multi-value prefixes, min/max/distinct and sharded counters, carts that expire,
 *   and notes that cascade-delete with their order.
 * - **HTTP:** declared response types, 201/204 and redirects, headers and cookies, errors with
 *   details, and CORS.
 * - **integrations:** a signed payment webhook with event dedupe, caller profiles and the user
 *   directory, keyed reminder jobs that are cancelled on payment, and a weekly calendar digest.
 * - **examples:** worked examples that `aleph test` runs.
 */
import { $, app, err, t, verify } from "@aleph/sdk";

const customerId = t.string().min(1).max(64);
const sku = t.string().min(1).max(32);

const a = app("commerce", {
  profile: { email: t.string().max(200), name: t.string().max(100) },
  cors: { origins: ["https://shop.example.com"], credentials: true, maxAge: "1h" },
});

// ─── Catalog ───────────────────────────────────────────────────────────────────────────────

const Product = a.entity("Product", {
  key: ["sku"],
  fields: {
    sku,
    name: t.string().min(1).max(80),
    price: t.decimal(2).min("0.00"),
    category: t.enum("books", "games", "tools"),
    tags: t.array(t.string().max(20), 5).optional(),
    version: t.version(),
  },
  versioned: true,
  indexes: { byCategory: ["category", "price"] },
});

const cheapest = a.counter("cheapest", {
  of: Product,
  groupBy: ["category"],
  agg: { min: "price" },
});

const productOut = t.object({
  sku,
  name: t.string().max(80),
  price: t.decimal(2),
  category: t.enum("books", "games", "tools"),
  version: t.int(),
});

a.route("PUT /products/:sku")
  .params({ sku })
  .body({
    name: t.string().min(1).max(80),
    price: t.decimal(2).min("0.00"),
    category: t.enum("books", "games", "tools"),
  })
  .returns(productOut)
  .describe("Create or replace a product")
  .example({
    name: "creates a product at version 1",
    as: { userId: "admin" },
    request: { params: { sku: "BK-1" }, body: { name: "Rust", price: "39.9", category: "books" } },
    expect: { body: { sku: "BK-1", price: "39.90", version: 1 } },
  })
  .handle((_, { params, body }) =>
    Product.upsert({
      sku: params.sku,
      name: body.name,
      price: body.price,
      category: body.category,
    }),
  );

a.route("PATCH /products/:sku/price")
  .params({ sku })
  .body({ price: t.decimal(2).min("0.00"), version: t.int().min(1) })
  .returns(productOut)
  .errors({ missing: err.notFound("No such product") })
  .example({
    name: "a stale version is rejected",
    as: { userId: "admin" },
    given: { Product: [{ sku: "BK-1", name: "Rust", price: "39.90", category: "books" }] },
    request: { params: { sku: "BK-1" }, body: { price: "35.00", version: 7 } },
    expect: { status: 409, body: { error: "version_conflict" } },
  })
  .example({
    name: "the current version updates and bumps it",
    as: { userId: "admin" },
    given: { Product: [{ sku: "BK-1", name: "Rust", price: "39.90", category: "books" }] },
    request: { params: { sku: "BK-1" }, body: { price: "35.00", version: 1 } },
    expect: { body: { price: "35.00", version: 2 } },
  })
  .handle((_, { params, body, errors }) => {
    const product = Product.getOrFail({ sku: params.sku }, errors.missing);
    return product.update({ price: body.price }, { ifVersion: body.version });
  });

a.route("GET /deals")
  .auth("public")
  .describe("The cheapest books and games, merged in price order")
  .example({
    name: "merges two categories by price",
    given: {
      Product: [
        { sku: "B1", name: "Book", price: "12.00", category: "books" },
        { sku: "G1", name: "Game", price: "9.50", category: "games" },
        { sku: "T1", name: "Tool", price: "1.00", category: "tools" },
        { sku: "B2", name: "Pricey", price: "80.00", category: "books" },
      ],
    },
    expect: {
      body: {
        deals: [{ sku: "G1" }, { sku: "B1" }, { sku: "B2" }],
        cheapestBook: "12.00",
        cheapestTool: "1.00",
      },
    },
  })
  .handle(() => ({
    deals: Product.query({ index: "byCategory", anyOf: ["books", "games"], limit: 5 }).map((p) => ({
      sku: p.sku,
      price: p.price,
    })),
    cheapestBook: cheapest.get({ category: "books" }),
    cheapestTool: cheapest.get({ category: "tools" }),
  }));

a.route("GET /go/:sku")
  .auth("public")
  .params({ sku })
  .handle(($, { params }) => {
    $.redirect($.fmt`/products/${params.sku}`, 301);
  });

// ─── Orders ────────────────────────────────────────────────────────────────────────────────

const Order = a.entity("Order", {
  key: ["customerId", "orderId"],
  fields: {
    customerId,
    orderId: t.id().generated(),
    items: t.array(t.object({ sku, qty: t.int().min(1).max(99), unitPrice: t.decimal(2) }), 10),
    shipTo: t.object({
      city: t.string().min(1).max(60),
      zip: t.string().min(1).max(10),
      note: t.string().max(100).optional(),
    }),
    total: t.decimal(2),
    status: t.enum("placed", "paid", "cancelled").default("placed"),
    tz: t.timeZone().default("UTC"),
    placedAt: t.timestamp().defaultNow(),
  },
  indexes: { byStatus: ["status", "placedAt"] },
});

const ordersPerDay = a.counter("ordersPerDay", {
  of: Order,
  groupBy: [{ name: "day", value: (o) => o.placedAt.startOf("day") }],
  shards: 4,
});
const customers = a.counter("customers", {
  of: Order,
  groupBy: [],
  agg: { distinct: "customerId" },
});
const latestOrder = a.counter("latestOrder", { of: Order, groupBy: [], agg: { max: "placedAt" } });

const Note = a.entity("Note", {
  key: ["customerId", "orderId", "noteId"],
  fields: { customerId, orderId: t.id(), noteId: t.id().generated(), text: t.string().max(200) },
  parent: {
    entity: Order,
    key: { customerId: "customerId", orderId: "orderId" },
    onDelete: "cascade",
  },
});

const Reminder = a.queue("reminder", {
  payload: { customerId, orderId: t.id() },
});

const mail = a.integration("mail", {
  provider: "email",
  operations: {
    send: { mode: "effect", input: { to: t.string().max(200), subject: t.string().max(200) } },
  },
});

a.onQueue("remind", Reminder, ($, { payload }) => {
  const order = Order.get({ customerId: payload.customerId, orderId: payload.orderId });
  const user = a.users.get(payload.customerId);
  $.when(order.status.eq("placed").and(user.email.isSet()), () => {
    mail.send("send", { to: user.email.ifNull(""), subject: "Your order is waiting for payment" });
  });
});

const lineOut = t.object({ sku, qty: t.int(), unitPrice: t.decimal(2), lineTotal: t.decimal(2) });

a.route("POST /orders")
  .body({
    items: t.array(t.object({ sku, qty: t.int().min(1).max(99) }), 10),
    shipTo: t.object({
      city: t.string().min(1).max(60),
      zip: t.string().min(1).max(10),
      note: t.string().max(100).optional(),
    }),
    tz: t.timeZone().default("UTC"),
  })
  .status(201)
  .errors({
    unknownProducts: err.unprocessable("Some products don't exist", { missing: t.int() }),
    empty: err.badRequest("An order needs at least one item"),
  })
  .returns(
    t.object({
      orderId: t.id(),
      total: t.decimal(2),
      lines: t.array(lineOut, 10),
      items: t.int(),
    }),
  )
  .example({
    name: "prices the items, totals exactly, and emails the customer",
    as: { userId: "c1", profile: { email: "ada@example.com", name: "Ada" } },
    given: {
      Product: [
        { sku: "BK-1", name: "Rust", price: "19.99", category: "books" },
        { sku: "GM-1", name: "Chess", price: "0.10", category: "games" },
      ],
    },
    request: {
      body: {
        items: [
          { sku: "BK-1", qty: 3 },
          { sku: "GM-1", qty: 2 },
        ],
        shipTo: { city: "Paris", zip: "75001" },
      },
    },
    process: true,
    expect: {
      status: 201,
      body: {
        total: "60.17",
        items: 5,
        lines: [{ lineTotal: "59.97" }, { lineTotal: "0.20" }],
      },
      deliveries: [
        { target: "mail", data: { to: "ada@example.com", subject: "Order total 60.17" } },
      ],
    },
  })
  .example({
    name: "unknown products fail with details",
    as: { userId: "c1" },
    given: { Product: [{ sku: "BK-1", name: "Rust", price: "19.99", category: "books" }] },
    request: {
      body: {
        items: [
          { sku: "BK-1", qty: 1 },
          { sku: "NOPE", qty: 1 },
          { sku: "GONE", qty: 2 },
        ],
        shipTo: { city: "Paris", zip: "75001" },
      },
    },
    expect: { status: 422, body: { error: "unknownProducts", details: { missing: 2 } } },
  })
  .example({
    name: "nested input problems name the exact field",
    as: { userId: "c1" },
    request: {
      body: { items: [{ sku: "BK-1", qty: 0 }], shipTo: { city: "Paris", zip: "75001" } },
    },
    expect: { status: 400, body: { details: [{ field: "body.items.0.qty" }] } },
  })
  .handle(($, { body, errors }) => {
    $.ensure(body.items.length.gt(0), errors.empty);
    const lines = body.items.map((item) => {
      const product = Product.get({ sku: item.sku });
      const unitPrice = product.price.ifNull($.decimal("0.00"));
      return {
        sku: item.sku,
        qty: item.qty,
        found: product.exists(),
        unitPrice,
        lineTotal: unitPrice.times(item.qty),
      };
    });
    $.ensure(
      lines.every((l) => l.found),
      errors.unknownProducts,
      {
        missing: lines.count((l) => l.found.not()),
      },
    );
    const priced = lines.map((l) => ({
      sku: l.sku,
      qty: l.qty,
      unitPrice: l.unitPrice,
      lineTotal: l.lineTotal.round(2),
    }));
    const total = priced.sum((l) => l.lineTotal).round(2);
    const order = Order.insert({
      customerId: $.caller,
      items: priced.map((l) => ({ sku: l.sku, qty: l.qty, unitPrice: l.unitPrice })),
      shipTo: body.shipTo,
      total,
      tz: body.tz,
    });
    // A reminder in an hour, keyed by order so paying can cancel it.
    Reminder.enqueue(
      { customerId: order.customerId, orderId: order.orderId },
      { delay: "1h", key: order.orderId },
    );
    $.when(a.caller.profile.email.isSet(), () => {
      mail.send("send", {
        to: a.caller.profile.email.ifNull(""),
        subject: $.fmt`Order total ${total}`,
      });
    });
    $.header("location", $.fmt`/orders/${order.orderId}`);
    $.log("total", total);
    return { orderId: order.orderId, total, lines: priced, items: priced.sum((l) => l.qty) };
  });

a.route("POST /orders/:orderId/pay")
  .params({ orderId: t.id() })
  .errors({ missing: err.notFound("No such order") })
  .handle(($, { params, errors }) => {
    const order = Order.getOrFail(
      { customerId: $.caller, orderId: params.orderId },
      errors.missing,
    );
    $.when(order.status.eq("paid"), () => $.return({ status: "paid", changed: false }));
    order.update({ status: "paid" });
    Reminder.cancel(order.orderId);
    return { status: "paid", changed: true };
  });

a.route("GET /orders/:orderId/receipt")
  .params({ orderId: t.id() })
  .errors({ missing: err.notFound("No such order") })
  .example({
    name: "formats a receipt in the order's time zone",
    as: { userId: "c1" },
    given: {
      Order: [
        {
          customerId: "c1",
          orderId: "01J00000000000000000000001",
          items: [{ sku: "BK-1", qty: 2, unitPrice: "1234.50" }],
          shipTo: { city: "Tokyo", zip: "100" },
          total: "2469.00",
          tz: "Asia/Tokyo",
          placedAt: 1_700_000_000_000,
        },
      ],
    },
    request: { params: { orderId: "01J00000000000000000000001" } },
    expect: {
      body: {
        date: "2023-11-15 Wed",
        weekday: 3,
        lines: ["BK-1     x2"],
        total: "2,469.00",
        dueBy: "2023-11-22",
        ship: "TOKYO 100",
      },
    },
  })
  .handle(($, { params, errors }) => {
    const order = Order.getOrFail(
      { customerId: $.caller, orderId: params.orderId },
      errors.missing,
    );
    return {
      date: order.placedAt.formatTime("%Y-%m-%d %a", order.tz),
      weekday: order.placedAt.part("weekday", order.tz),
      lines: order.items.map((i) => $.fmt`${i.sku.padEnd(8)} x${i.qty}`),
      total: order.total.format({ decimals: 2, grouping: true }),
      dueBy: order.placedAt.add(1, "week", order.tz).formatTime("%Y-%m-%d", order.tz),
      ship: $.fmt`${order.shipTo.city.upper()} ${order.shipTo.zip}`,
    };
  });

a.route("GET /stats")
  .auth("public")
  .handle(() => ({
    customers: customers.get({}),
    latest: latestOrder.get({}),
    today: ordersPerDay.get({ day: $.now().startOf("day") }),
  }));

a.route("POST /orders/:orderId/notes")
  .params({ orderId: t.id() })
  .body({ text: t.string().min(1).max(200) })
  .status(201)
  .handle(($, { params, body }) =>
    Note.insert({ customerId: $.caller, orderId: params.orderId, text: body.text }),
  );

a.route("GET /orders/:orderId/notes")
  .params({ orderId: t.id() })
  .errors({ missing: err.notFound("No such order") })
  .handle(($, { params, errors }) => {
    const order = Order.getOrFail(
      { customerId: $.caller, orderId: params.orderId },
      errors.missing,
    );
    return { notes: Note.childrenOf(order, { limit: 20 }).map((n) => n.text) };
  });

a.route("DELETE /orders/:orderId")
  .params({ orderId: t.id() })
  .status(204)
  .handle(($, { params }) => {
    Order.delete({ customerId: $.caller, orderId: params.orderId });
  });

// ─── Payments webhook ──────────────────────────────────────────────────────────────────────

a.webhook("POST /webhooks/payments", {
  verify: verify.hmacSha256({ secret: "PAYMENTS_SECRET", header: "x-signature" }),
  dedupe: { bodyField: "eventId", ttl: "1d" },
})
  .body({ eventId: t.string().max(64), customerId, orderId: t.id() })
  .example({
    name: "a signed delivery marks the order paid",
    secrets: { PAYMENTS_SECRET: "whsec_test" },
    given: {
      Order: [
        {
          customerId: "c1",
          orderId: "01J00000000000000000000001",
          items: [],
          shipTo: { city: "Oslo", zip: "0150" },
          total: "0.00",
        },
      ],
    },
    request: {
      headers: {
        "x-signature": "eefc5edda89b6b864c99347c5a885d437939d3fa4e45c72912aa3196a2350a3e",
      },
      body: { customerId: "c1", eventId: "evt_1", orderId: "01J00000000000000000000001" },
    },
    expect: { body: { paid: true } },
  })
  .example({
    name: "a forged delivery is rejected",
    secrets: { PAYMENTS_SECRET: "whsec_test" },
    request: {
      headers: { "x-signature": "00" },
      body: { customerId: "c1", eventId: "evt_1", orderId: "01J00000000000000000000001" },
    },
    expect: { status: 401, body: { error: "invalid_signature" } },
  })
  .handle(($, { body }) => {
    const order = Order.get({ customerId: body.customerId, orderId: body.orderId });
    $.when(order.exists(), () => {
      order.update({ status: "paid" });
      Reminder.cancel(body.orderId);
    });
    return { paid: order.exists() };
  });

// ─── Profiles, carts, and the rest ─────────────────────────────────────────────────────────

a.route("GET /me")
  .example({
    name: "reads the caller's profile and the directory",
    as: { userId: "c1", profile: { email: "ada@example.com", name: "Ada" } },
    users: { c1: { email: "ada@directory.example", name: "Ada L." } },
    expect: {
      body: { id: "c1", email: "ada@example.com", directoryName: "Ada L.", greeting: "Hi, Ada" },
    },
  })
  .handle(($) => {
    const me = a.users.get($.caller);
    $.cookie("seen", "1", { maxAge: 3600 });
    return {
      id: $.caller,
      email: a.caller.profile.email,
      directoryName: me.name,
      greeting: $.fmt`Hi, ${a.caller.profile.name}`,
    };
  });

const Cart = a.entity("Cart", {
  key: ["customerId"],
  fields: {
    customerId,
    skus: t.array(sku, 10),
    updatedAt: t.timestamp().defaultNow(),
  },
  ttl: { field: "updatedAt", after: "1d" },
});

a.route("PUT /cart")
  .body({ skus: t.array(sku, 10) })
  .handle(($, { body }) =>
    Cart.upsert({ customerId: $.caller, skus: body.skus.distinct(), updatedAt: $.now() }),
  );

a.route("GET /cart").handle(($) => {
  const cart = Cart.get({ customerId: $.caller });
  return { exists: cart.exists(), skus: cart.skus, first: cart.skus.sortBy((s) => s).first() };
});

a.route("GET /legacy/products")
  .auth("public")
  .deprecated("Use GET /deals")
  .version("2024-01-01")
  .handle(() => ({ products: Product.query({ limit: 10 }).map((p) => p.sku) }));

a.every(
  "weeklyDigest",
  "mon 09:00",
  ($) => {
    $.log("digest", latestOrder.get({}));
  },
  { tz: "Europe/Berlin" },
);

export default a;
