/**
 * React helpers for the examples: call the app as a chosen caller, and inspect what happened.
 *
 * - `<AlephProvider presets={...}>` holds the current caller (the dev server fakes auth with
 *   headers) and injects the example styles.
 * - `useAleph().call(method, path, { body, query, idempotencyKey })` calls the app through `/api`.
 * - `<Inspector />` shows each request's route, status, and actual usage against its static
 *   bounds, the async activity (handler runs, retries, deliveries, integration calls), and the
 *   app's worst-case usage report.
 */
import type { CheckReport, Cost } from "@aleph-lang/manifest";
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";

/** Who the dev server should treat as the caller. */
export interface Caller {
  label: string;
  userId?: string;
  /** Org roles, for example `{ acme: "owner" }`. */
  orgs?: Record<string, string>;
  /** An API key for an org, with scopes. */
  apiKey?: { orgId: string; scopes: string[] };
}

/** The result of a call. */
export interface CallResult<T = unknown> {
  status: number;
  ok: boolean;
  body: T;
  route?: string;
  usage?: Cost;
  replayed: boolean;
}

/** Options for {@link AlephContextValue.call}. */
export interface CallOptions {
  body?: unknown;
  query?: Record<string, string | number | boolean | undefined>;
  idempotencyKey?: string;
}

interface AlephContextValue {
  caller: Caller;
  setCaller: (caller: Caller) => void;
  presets: Caller[];
  /** Bumps after every call, so lists can refetch. */
  version: number;
  call: <T = unknown>(
    method: string,
    path: string,
    options?: CallOptions,
  ) => Promise<CallResult<T>>;
}

const AlephContext = createContext<AlephContextValue | undefined>(undefined);

function headers(caller: Caller): Record<string, string> {
  const out: Record<string, string> = { "content-type": "application/json" };
  if (caller.userId) out["x-aleph-user"] = caller.userId;
  if (caller.orgs) {
    out["x-aleph-orgs"] = Object.entries(caller.orgs)
      .map(([org, role]) => `${org}=${role}`)
      .join(",");
  }
  if (caller.apiKey) {
    out["x-aleph-api-key"] = caller.apiKey.orgId;
    out["x-aleph-api-key-scopes"] = caller.apiKey.scopes.join(",");
  }
  return out;
}

/** Provides the caller and `call()` to the example UI. */
export function AlephProvider({ presets, children }: { presets: Caller[]; children: ReactNode }) {
  const [caller, setCaller] = useState<Caller>(presets[0] ?? { label: "anonymous" });
  const [version, setVersion] = useState(0);

  const call = useCallback(
    async <T,>(method: string, path: string, options: CallOptions = {}): Promise<CallResult<T>> => {
      const query = new URLSearchParams();
      for (const [key, value] of Object.entries(options.query ?? {})) {
        if (value !== undefined && value !== "") query.set(key, String(value));
      }
      const search = query.size > 0 ? `?${query}` : "";
      const init: RequestInit = { method, headers: headers(caller) };
      if (options.idempotencyKey) {
        init.headers = { ...init.headers, "idempotency-key": options.idempotencyKey };
      }
      if (options.body !== undefined) init.body = JSON.stringify(options.body);
      const response = await fetch(`/api${path}${search}`, init);
      const usage = response.headers.get("x-aleph-usage");
      const route = response.headers.get("x-aleph-route");
      setVersion((v) => v + 1);
      return {
        status: response.status,
        ok: response.ok,
        body: (await response.json()) as T,
        ...(route ? { route } : {}),
        ...(usage ? { usage: JSON.parse(usage) as Cost } : {}),
        replayed: response.headers.get("x-aleph-replayed") === "true",
      };
    },
    [caller],
  );

  const value = useMemo(
    () => ({ caller, setCaller, presets, version, call }),
    [caller, presets, version, call],
  );
  return (
    <AlephContext.Provider value={value}>
      <style>{styles}</style>
      {children}
    </AlephContext.Provider>
  );
}

/** The current caller and `call()`. */
export function useAleph(): AlephContextValue {
  const value = useContext(AlephContext);
  if (!value) throw new Error("useAleph() must be used inside <AlephProvider>");
  return value;
}

/** Fetches `path` as the current caller whenever anything changes, and every `pollMs`. */
export function useQuery<T>(
  path: string,
  query?: CallOptions["query"],
  pollMs?: number,
): CallResult<T> | undefined {
  const { call, version, caller } = useAleph();
  const [result, setResult] = useState<CallResult<T>>();
  const [tick, setTick] = useState(0);
  const key = JSON.stringify(query ?? {});
  useEffect(() => {
    if (!pollMs) return;
    const timer = setInterval(() => setTick((t) => t + 1), pollMs);
    return () => clearInterval(timer);
  }, [pollMs]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: refetch on caller, version, poll ticks, and query changes
  useEffect(() => {
    let live = true;
    // Reads shouldn't bump `version`, or every fetch would trigger another one.
    fetch(`/api${path}${query ? `?${new URLSearchParams(clean(query))}` : ""}`, {
      headers: headers(caller),
    })
      .then(async (response) => {
        const usage = response.headers.get("x-aleph-usage");
        const route = response.headers.get("x-aleph-route");
        const body = (await response.json()) as T;
        if (live) {
          setResult({
            status: response.status,
            ok: response.ok,
            body,
            ...(route ? { route } : {}),
            ...(usage ? { usage: JSON.parse(usage) as Cost } : {}),
            replayed: false,
          });
        }
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [path, key, caller, version, tick, call]);
  return result;
}

function clean(query: NonNullable<CallOptions["query"]>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(query)
      .filter(([, v]) => v !== undefined && v !== "")
      .map(([k, v]) => [k, String(v)]),
  );
}

/** Sets an integration's mock response (or failure) in the dev engine. */
export async function mock(
  integration: string,
  operation: string,
  response: { ok: unknown } | { error: string },
) {
  const body =
    "error" in response
      ? { integration, operation, error: response.error }
      : { integration, operation, response: response.ok };
  await fetch("/__aleph/mock", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

// ─── Inspector ──────────────────────────────────────────────────────────────────────────────────

// The dev server's activity feed (requests, runs, deliveries, logs, reloads).
type Activity = import("@aleph-lang/cli").Activity;

type ReportResult = { ok: true; report: CheckReport } | { ok: false };

const RESOURCES: [keyof Cost, string][] = [
  ["reads", "reads"],
  ["writes", "writes"],
  ["ops", "ops"],
  ["bytesRead", "bytes"],
  ["effects", "effects"],
  ["calls", "calls"],
  ["memory", "memory"],
];

function useActivity(): Activity[] {
  const [items, setItems] = useState<Activity[]>([]);
  useEffect(() => {
    let after = 0;
    let live = true;
    const poll = async () => {
      try {
        const response = await fetch(`/__aleph/activity?after=${after}`);
        const { activity } = (await response.json()) as { activity: Activity[] };
        const last = activity.at(-1);
        if (live && last) {
          after = last.seq;
          setItems((prev) => [...prev, ...activity].slice(-200));
        }
      } catch {
        // the dev server is restarting
      }
    };
    poll();
    const timer = setInterval(poll, 700);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, []);
  return items;
}

function useReport(): CheckReport | undefined {
  const [report, setReport] = useState<CheckReport>();
  useEffect(() => {
    fetch("/__aleph/report")
      .then((r) => r.json() as Promise<ReportResult>)
      .then((r) => (r.ok ? setReport(r.report) : undefined))
      .catch(() => {});
  }, []);
  return report;
}

function UsageBars({ usage, bounds }: { usage: Cost; bounds?: Cost | undefined }) {
  return (
    <span className="al-usage">
      {RESOURCES.filter(([key]) => usage[key] > 0 || (bounds?.[key] ?? 0) > 0).map(
        ([key, label]) => (
          <span
            key={key}
            title={`${label}: ${usage[key]} used of ${bounds?.[key] ?? "?"} worst case`}
          >
            {label} {usage[key]}
            {bounds ? <span className="al-dim">/{bounds[key]}</span> : null}
          </span>
        ),
      )}
    </span>
  );
}

function ActivityRow({ item, report }: { item: Activity; report?: CheckReport | undefined }) {
  switch (item.kind) {
    case "request":
      return (
        <li>
          <span className={item.status < 400 ? "al-ok" : "al-err"}>{item.status}</span>{" "}
          {item.method} {item.path}
          <span className="al-dim"> {item.route ?? "no route"}</span>
          {item.replayed ? <span className="al-tag">replayed</span> : null}
          {item.attempts > 1 ? (
            <span className="al-tag al-warn">hot: {item.attempts - 1} conflict retries</span>
          ) : null}
          <UsageBars
            usage={item.usage}
            bounds={item.route ? report?.routes[item.route]?.bounds : undefined}
          />
          {item.logs.map((entry, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: logs are append-only per request.
            <div key={i} className="al-indent al-dim">
              log <b>{entry.label}</b> <code>{JSON.stringify(entry.value)}</code>
            </div>
          ))}
        </li>
      );
    case "reload":
      return (
        <li className={item.ok ? "al-ok" : "al-err"}>
          ↻ {item.ok ? item.message : `reload failed: ${item.message}`}
        </li>
      );
    case "run":
      return (
        <li className="al-indent">
          ↳ ran <b>{item.handler}</b>
          <UsageBars usage={item.usage} bounds={report?.handlers?.[item.handler]?.bounds} />
        </li>
      );
    case "retry":
      return (
        <li className="al-indent al-warn">
          ↳ {item.count} failed attempt(s), retrying with backoff
        </li>
      );
    case "failure":
      return <li className="al-indent al-err">↳ {item.message}</li>;
    case "delivery":
      return (
        <li className="al-indent">
          ↳ {item.delivery.kind}{" "}
          <b>
            {item.delivery.target}
            {item.delivery.operation ? `.${item.delivery.operation}` : ""}
          </b>{" "}
          <code>{JSON.stringify(item.delivery.data)}</code>
        </li>
      );
    case "call":
      return (
        <li className="al-indent">
          ↳ call{" "}
          <b>
            {item.call.integration}.{item.call.operation}
          </b>{" "}
          <code>{JSON.stringify(item.call.input)}</code>
        </li>
      );
  }
}

function BoundsTable({
  title,
  reports,
}: {
  title: string;
  reports: Record<string, { bounds: Cost }>;
}) {
  return (
    <table className="al-table">
      <thead>
        <tr>
          <th>{title}</th>
          {RESOURCES.map(([key, label]) => (
            <th key={key}>{label}</th>
          ))}
        </tr>
      </thead>
      <tbody>
        {Object.entries(reports).map(([id, { bounds }]) => (
          <tr key={id}>
            <td>{id}</td>
            {RESOURCES.map(([key]) => (
              <td key={key}>{bounds[key]}</td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** The inspector panel: caller switcher, activity feed, and static bounds. */
export function Inspector() {
  const { caller, setCaller, presets } = useAleph();
  const activity = useActivity();
  const report = useReport();
  const [tab, setTab] = useState<"activity" | "bounds">("activity");
  return (
    <aside className="al-inspector">
      <h2>Aleph inspector</h2>
      <div className="al-row">
        <span className="al-dim">Call as</span>
        {presets.map((preset) => (
          <button
            type="button"
            key={preset.label}
            className={preset.label === caller.label ? "al-active" : ""}
            onClick={() => setCaller(preset)}
          >
            {preset.label}
          </button>
        ))}
      </div>
      <div className="al-row">
        <button
          type="button"
          className={tab === "activity" ? "al-active" : ""}
          onClick={() => setTab("activity")}
        >
          Activity
        </button>
        <button
          type="button"
          className={tab === "bounds" ? "al-active" : ""}
          onClick={() => setTab("bounds")}
        >
          Static bounds
        </button>
      </div>
      {tab === "activity" ? (
        <ul className="al-activity">
          {[...activity].reverse().map((item) => (
            <ActivityRow key={item.seq} item={item} report={report} />
          ))}
        </ul>
      ) : report ? (
        <>
          <p className="al-dim">The most one execution can use, computed before anything runs.</p>
          <BoundsTable title="route" reports={report.routes} />
          {report.handlers ? <BoundsTable title="handler" reports={report.handlers} /> : null}
        </>
      ) : null}
    </aside>
  );
}

/** A labeled JSON view of a call result, for the example pages. */
export function ResultView({ result }: { result?: CallResult | undefined }) {
  if (!result) return null;
  return (
    <pre className={result.ok ? "al-result" : "al-result al-result-err"}>
      {result.status} {JSON.stringify(result.body, null, 2)}
    </pre>
  );
}

const styles = `
  :root { font-family: ui-sans-serif, system-ui, sans-serif; color: #1d1d1f; }
  body { margin: 0; }
  .al-layout { display: grid; grid-template-columns: minmax(0, 1fr) 460px; min-height: 100vh; }
  .al-main { padding: 24px 32px; }
  .al-main h1 { margin-top: 0; }
  .al-card { border: 1px solid #ddd; border-radius: 8px; padding: 12px 16px; margin-bottom: 16px; }
  .al-card h3 { margin: 0 0 8px; font-size: 15px; }
  .al-row { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; margin: 6px 0; }
  .al-main input, .al-main select { padding: 5px 8px; border: 1px solid #ccc; border-radius: 6px; }
  button { padding: 5px 10px; border: 1px solid #bbb; border-radius: 6px; background: #fff; cursor: pointer; }
  button:hover { background: #f2f2f2; }
  button.al-active { background: #1d1d1f; color: #fff; border-color: #1d1d1f; }
  .al-list { list-style: none; padding: 0; margin: 0; }
  .al-list li { display: flex; gap: 8px; align-items: center; padding: 4px 0; border-bottom: 1px solid #f0f0f0; }
  .al-inspector { background: #111; color: #e8e8e8; padding: 16px; font-size: 12px; overflow-y: auto; max-height: 100vh; position: sticky; top: 0; }
  .al-inspector h2 { margin: 0 0 8px; font-size: 14px; }
  .al-inspector button { background: #222; color: #ddd; border-color: #444; font-size: 12px; }
  .al-inspector button.al-active { background: #e8e8e8; color: #111; }
  .al-activity { list-style: none; padding: 0; margin: 8px 0; font-family: ui-monospace, monospace; }
  .al-activity li { padding: 3px 0; border-bottom: 1px solid #222; }
  .al-indent { padding-left: 12px !important; color: #bbb; }
  .al-usage { display: block; color: #8fd; }
  .al-usage span { margin-right: 8px; }
  .al-dim { color: #888; }
  .al-ok { color: #7d7; } .al-err { color: #f77; } .al-warn { color: #fc6; }
  .al-tag { margin-left: 6px; padding: 0 4px; border: 1px solid #fc6; color: #fc6; border-radius: 4px; }
  .al-table { border-collapse: collapse; width: 100%; margin-bottom: 12px; font-family: ui-monospace, monospace; }
  .al-table th, .al-table td { text-align: right; padding: 2px 4px; border-bottom: 1px solid #222; }
  .al-table th:first-child, .al-table td:first-child { text-align: left; }
  .al-result { background: #f6f6f6; padding: 8px; border-radius: 6px; font-size: 12px; overflow-x: auto; }
  .al-result-err { background: #fff0f0; }
  code { font-size: 11px; color: #ccc; }
`;
