/**
 * `aleph dev`: a local HTTP server on the WASM dev engine, the same core the platform runs.
 *
 * Authentication is faked with headers, since the platform is what authenticates real callers:
 * - `x-aleph-user: u1`
 * - `x-aleph-orgs: acme=owner,other=member`
 * - `x-aleph-api-key: acme` plus `x-aleph-api-key-scopes: projects:read,projects:write`
 * - `idempotency-key: <key>`
 */
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { DevEngine, validate } from "@aleph/core";
import type { Caller, Cost, Delivery, Method, RecordedCall, Request } from "@aleph/manifest";

const METHODS = new Set<string>(["GET", "POST", "PUT", "PATCH", "DELETE"]);

function header(req: IncomingMessage, name: string): string | undefined {
  const value = req.headers[name];
  return Array.isArray(value) ? value[0] : value;
}

/** Builds the caller from the dev auth headers, or `undefined` if there are none. */
export function callerFromHeaders(req: IncomingMessage): Caller | undefined {
  const user = header(req, "x-aleph-user");
  const orgs = header(req, "x-aleph-orgs");
  const key = header(req, "x-aleph-api-key");
  if (!user && !key) return undefined;
  const caller: Caller = {};
  if (user) caller.userId = user;
  if (orgs) {
    caller.orgs = Object.fromEntries(
      orgs.split(",").flatMap((pair) => {
        const [org, role] = pair.split("=").map((s) => s.trim());
        return org && role ? [[org, role]] : [];
      }),
    );
  }
  if (key) {
    const scopes = header(req, "x-aleph-api-key-scopes") ?? "";
    caller.apiKey = {
      orgId: key,
      scopes: scopes
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
    };
  }
  return caller;
}

async function readBody(req: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) chunks.push(chunk as Buffer);
  const text = Buffer.concat(chunks).toString("utf8").trim();
  return text === "" ? undefined : (JSON.parse(text) as unknown);
}

function send(
  res: ServerResponse,
  status: number,
  body: unknown,
  headers: Record<string, string> = {},
): void {
  res.writeHead(status, { "content-type": "application/json", ...headers });
  res.end(`${JSON.stringify(body)}\n`);
}

/** One thing that happened in the dev server, for the inspector. */
export type Activity = { seq: number; at: number } & (
  | {
      kind: "request";
      method: string;
      path: string;
      status: number;
      route?: string;
      usage: Cost;
      replayed: boolean;
      ms: number;
      body: unknown;
    }
  | { kind: "run"; handler: string; usage: Cost }
  | { kind: "retry"; count: number }
  | { kind: "failure"; message: string }
  | { kind: "delivery"; delivery: Delivery }
  | { kind: "call"; call: RecordedCall }
);

type ActivityEvent = Activity extends infer A
  ? A extends Activity
    ? Omit<A, "seq" | "at">
    : never
  : never;

/** A canned integration response for the dev engine. */
export interface MockResponse {
  integration: string;
  operation: string;
  response?: unknown;
  error?: string;
}

/** Options for {@link startDevServer}. */
export interface DevServerOptions {
  port?: number;
  host?: string;
  /** How often to run due async work (cron, handlers, deliveries), in ms. Default 1000. */
  processEveryMs?: number;
  /** Integration responses to start with (change them later with `POST /__aleph/mock`). */
  mocks?: MockResponse[];
  /** Called once per request with a one-line summary. */
  log?: (line: string) => void;
}

const MAX_ACTIVITY = 500;

/**
 * Starts the dev server. Resolves once it's listening.
 *
 * Besides the app's routes, it serves an inspection API under `/__aleph/`:
 * - `GET /__aleph/manifest` and `GET /__aleph/report` (static bounds)
 * - `GET /__aleph/activity?after=<seq>`: requests, handler runs, deliveries, calls
 * - `POST /__aleph/mock` `{ integration, operation, response | error }`
 * - `POST /__aleph/process`: run due async work now
 */
export async function startDevServer(
  manifestJson: string,
  options: DevServerOptions = {},
): Promise<Server> {
  const engine = new DevEngine(manifestJson, { seed: Date.now() });
  const validation = validate(manifestJson);
  const log = options.log ?? (() => {});
  const activity: Activity[] = [];
  let seq = 0;
  const record = (event: ActivityEvent) => {
    activity.push({ ...event, seq: ++seq, at: Date.now() } as Activity);
    if (activity.length > MAX_ACTIVITY) activity.splice(0, activity.length - MAX_ACTIVITY);
  };
  for (const mock of options.mocks ?? []) {
    engine.mock(
      mock.integration,
      mock.operation,
      mock.error === undefined ? mock.response : { error: mock.error },
    );
  }

  // Async work runs right after each request and on a timer, like the platform's workers.
  function processDue(): void {
    const report = engine.process();
    for (const call of engine.takeCalls()) record({ kind: "call", call });
    for (const run of report.runs) {
      record({ kind: "run", handler: run.handler, usage: run.usage });
      log(`  ↳ ran ${run.handler} (reads ${run.usage.reads}, writes ${run.usage.writes})`);
    }
    for (const message of report.failures) {
      record({ kind: "failure", message });
      log(`  ↳ failed: ${message}`);
    }
    if (report.retried > 0) {
      record({ kind: "retry", count: report.retried });
      log(`  ↳ retried ${report.retried} (with backoff)`);
    }
    for (const delivery of engine.takeDeliveries()) {
      record({ kind: "delivery", delivery });
      const target = `${delivery.target}${delivery.operation ? `.${delivery.operation}` : ""}`;
      log(`  ↳ ${delivery.kind} ${target} ${JSON.stringify(delivery.data)}`);
    }
  }

  /** The inspection API's response for a request, or `undefined` if there's no such endpoint. */
  function inspect(method: string, url: URL, body: unknown): unknown {
    switch (`${method} ${url.pathname}`) {
      case "GET /__aleph/manifest":
        return JSON.parse(manifestJson);
      case "GET /__aleph/report":
        return validation;
      case "GET /__aleph/activity": {
        const after = Number(url.searchParams.get("after") ?? 0);
        return { activity: activity.filter((a) => a.seq > after) };
      }
      case "POST /__aleph/process":
        processDue();
        return { ok: true };
      case "POST /__aleph/mock": {
        const mock = body as MockResponse;
        const response = mock.error === undefined ? mock.response : { error: mock.error };
        engine.mock(mock.integration, mock.operation, response);
        return { ok: true };
      }
      default:
        return undefined;
    }
  }

  const server = createServer((req, res) => {
    const started = performance.now();
    const url = new URL(req.url ?? "/", "http://localhost");
    const method = (req.method ?? "GET").toUpperCase();
    if (!METHODS.has(method)) {
      send(res, 405, { error: "method_not_allowed", message: `${method} isn't supported` });
      return;
    }
    readBody(req)
      .then((body) => {
        if (url.pathname.startsWith("/__aleph/")) {
          const result = inspect(method, url, body);
          if (result === undefined) {
            send(res, 404, {
              error: "not_found",
              message: `no inspection endpoint ${url.pathname}`,
            });
          } else {
            send(res, 200, result);
          }
          return;
        }
        const request: Request = {
          method: method as Method,
          path: url.pathname,
          query: Object.fromEntries(url.searchParams),
        };
        const caller = callerFromHeaders(req);
        if (caller) request.caller = caller;
        if (body !== undefined) request.body = body;
        const key = header(req, "idempotency-key");
        if (key) request.idempotencyKey = key;

        const response = engine.handle(request);
        const ms = Number((performance.now() - started).toFixed(2));
        record({
          kind: "request",
          method,
          path: url.pathname,
          status: response.status,
          ...(response.route ? { route: response.route } : {}),
          usage: response.usage,
          replayed: response.replayed ?? false,
          ms,
          body: response.body,
        });
        processDue();
        const usage = response.usage;
        send(res, response.status, response.body, {
          "x-aleph-usage": JSON.stringify(usage),
          ...(response.route ? { "x-aleph-route": response.route } : {}),
          ...(response.replayed ? { "x-aleph-replayed": "true" } : {}),
        });
        log(
          `${method} ${url.pathname} → ${response.status} ${response.route ?? "-"} ` +
            `(reads ${usage.reads}, writes ${usage.writes}, ops ${usage.ops}) ${ms}ms`,
        );
      })
      .catch(() => send(res, 400, { error: "invalid_json", message: "the body isn't valid JSON" }));
  });
  const timer = setInterval(processDue, options.processEveryMs ?? 1000);
  timer.unref();
  await new Promise<void>((resolve) =>
    server.listen(options.port ?? 3000, options.host ?? "127.0.0.1", resolve),
  );
  server.on("close", () => {
    clearInterval(timer);
    engine.dispose();
  });
  return server;
}
