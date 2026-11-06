import {
  type CallResult,
  Inspector,
  mock,
  ResultView,
  useAleph,
  useQuery,
} from "@aleph-examples/inspector/react";
import { useState } from "react";
// Generated from app.ts by `pnpm client`: changing a route's response breaks this file's types.
import type { Routes } from "./client.ts";

type Orders = Routes["myOrders"]["output"];

const money = (cents: number) => `$${(cents / 100).toFixed(2)}`;

export function App() {
  const { call } = useAleph();
  const [last, setLast] = useState<CallResult>();
  const [amount, setAmount] = useState("25.00");
  const [orderKey, setOrderKey] = useState(() => crypto.randomUUID());
  const [payments, setPayments] = useState<"ok" | "down">("ok");
  const orders = useQuery<Orders>("/orders", undefined, 1000);

  async function place(sameKey: boolean) {
    const key = sameKey ? orderKey : crypto.randomUUID();
    setOrderKey(key);
    const totalCents = Math.round(Number(amount) * 100);
    setLast(await call("POST", "/orders", { body: { totalCents }, idempotencyKey: key }));
  }

  async function setProvider(state: "ok" | "down") {
    await mock(
      "payments",
      "charge",
      state === "ok"
        ? { ok: { chargeId: `ch_dev_${Date.now()}` } }
        : { error: "card network unavailable" },
    );
    setPayments(state);
  }

  return (
    <div className="al-layout">
      <main className="al-main">
        <h1>Shop</h1>
        <p>
          Placing an order enqueues a charge job. The job calls the payment integration, marks the
          order paid, and emits an event that sends a receipt email. Watch it in the inspector.
        </p>

        <section className="al-card">
          <h3>Payment provider (a mock in the dev engine)</h3>
          <div className="al-row">
            <button
              type="button"
              className={payments === "ok" ? "al-active" : ""}
              onClick={() => setProvider("ok")}
            >
              Charges succeed
            </button>
            <button
              type="button"
              className={payments === "down" ? "al-active" : ""}
              onClick={() => setProvider("down")}
            >
              Provider down
            </button>
          </div>
          <p className="al-dim">
            When it's down, the job retries with backoff (2s, 4s), then the dead-letter handler
            marks the order failed.
          </p>
        </section>

        <section className="al-card">
          <h3>Place an order</h3>
          <div className="al-row">
            <input aria-label="Amount" value={amount} onChange={(e) => setAmount(e.target.value)} />
            <button type="button" onClick={() => place(false)}>
              Place order
            </button>
            <button type="button" onClick={() => place(true)}>
              Retry the same request
            </button>
          </div>
          <p className="al-dim">
            Orders need an idempotency key. A retry with the same key replays the stored response,
            so it never creates (or charges) a second order. 10 orders a minute per user.
          </p>
          <ResultView result={last} />
        </section>

        <section className="al-card">
          <h3>
            My orders{" "}
            <span className="al-dim">
              (shop revenue {money(orders?.ok ? orders.body.revenueCents : 0)}, a running sum)
            </span>
          </h3>
          <ul className="al-list">
            {orders?.ok
              ? orders.body.page.items.map((o) => (
                  <li key={o.orderId}>
                    <span style={{ flex: 1 }}>{money(o.totalCents)}</span>
                    <span
                      className={
                        o.status === "paid" ? "al-ok" : o.status === "failed" ? "al-err" : "al-dim"
                      }
                    >
                      {o.status}
                    </span>
                    <span className="al-dim">{o.chargeId ?? ""}</span>
                  </li>
                ))
              : null}
          </ul>
        </section>
      </main>
      <Inspector />
    </div>
  );
}
