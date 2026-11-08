import { app, t } from "@aleph-lang/sdk";

const a = app("crm");

const Invoice = a.entity("Invoice", {
  key: ["customerId", "invoiceId"],
  fields: {
    customerId: t.string().max(64),
    invoiceId: t.id().generated(),
    amount: t.decimal(2),
  },
});

a.route("GET /invoices")
  .example({
    name: "anonymous callers are turned away",
    request: {},
    expect: { status: 401 },
  })
  .handle((_) => Invoice.page({ limit: 20 }));

export default a;
