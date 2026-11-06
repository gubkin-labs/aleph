/**
 * `aleph dev`: a local HTTP server on the WASM dev engine, the same core the platform runs.
 *
 * Authentication is faked with headers, since the platform is what authenticates real callers:
 * - `x-aleph-user: u1`
 * - `x-aleph-orgs: acme=owner,other=member`
 * - `x-aleph-api-key: acme` plus `x-aleph-api-key-scopes: projects:read,projects:write`
 * - `x-aleph-profile: {"email":"ada@example.com"}` (the caller's profile fields)
 * - `idempotency-key: <key>`
 */
import { existsSync, mkdirSync, readFileSync, watch, writeFileSync } from "node:fs";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { dirname } from "node:path";
import { DevEngine, validate } from "@aleph/core";
import type {
  Caller,
  Cost,
  Delivery,
  LogEntry,
  Method,
  RecordedCall,
  Request,
} from "@aleph/manifest";

const METHODS = new Set<string>(["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]);

/** Headers the dev server passes to the engine (for webhooks and CORS). */
const SKIPPED_HEADERS = new Set(["cookie", "authorization"]);

function header(req: IncomingMessage, name: string): string | undefined {
  const value = req.headers[name];
  return Array.isArray(value) ? value[0] : value;
}

/** Builds the caller from the dev auth headers, or `undefined` if there are none. */
export function callerFromHeaders(req: IncomingMessage): Caller | undefined {
  const user = header(req, "x-aleph-user");
  const orgs = header(req, "x-aleph-orgs");
  const key = header(req, "x-aleph-api-key");
  const profile = header(req, "x-aleph-profile");
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
  if (profile) {
    try {
      caller.profile = JSON.parse(profile) as Record<string, unknown>;
    } catch {
      // An unreadable profile header is ignored, like a missing one.
    }
  }
  return caller;
}

async function readBody(req: IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) chunks.push(chunk as Buffer);
  return Buffer.concat(chunks).toString("utf8");
}

function send(
  res: ServerResponse,
  status: number,
  body: unknown,
  headers: Record<string, string | string[]> = {},
): void {
  if (status === 204 || (status >= 300 && status < 400 && body === null)) {
    res.writeHead(status, headers);
    res.end();
    return;
  }
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
      attempts: number;
      ms: number;
      body: unknown;
      logs: LogEntry[];
    }
  | { kind: "run"; handler: string; usage: Cost }
  | { kind: "retry"; count: number }
  | { kind: "failure"; message: string }
  | { kind: "delivery"; delivery: Delivery }
  | { kind: "call"; call: RecordedCall }
  | { kind: "reload"; ok: boolean; message: string }
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

/** Seed data: rows by entity name, and optionally user profiles by id under `users`. */
export type SeedData = Record<string, unknown>;

/** Options for {@link startDevServer}. */
export interface DevServerOptions {
  port?: number;
  host?: string;
  /** How often to run due async work (cron, handlers, deliveries), in ms. Default 1000. */
  processEveryMs?: number;
  /** Integration responses to start with (change them later with `POST /__aleph/mock`). */
  mocks?: MockResponse[];
  /** Secret values (webhook signing keys), by name. */
  secrets?: Record<string, string>;
  /** Rows to insert at start (skipped where the key exists), and `users` profiles. */
  seed?: SeedData;
  /** Keep the data in this file between restarts (written after every change). */
  dataFile?: string;
  /** Rebuild the manifest when files next to `file` change, keeping the data. */
  watch?: { file: string; reload: () => Promise<string> };
  /** Called once per request with a one-line summary. */
  log?: (line: string) => void;
}

const MAX_ACTIVITY = 500;

/**
 * Starts the dev server. Resolves once it's listening.
 *
 * Besides the app's routes, it serves an inspection API under `/__aleph/`:
 * - `GET /__aleph/manifest` and `GET /__aleph/report` (static bounds)
 * - `GET /__aleph/activity?after=<seq>`: requests, handler runs, deliveries, calls, logs
 * - `POST /__aleph/mock` `{ integration, operation, response | error }`
 * - `POST /__aleph/process`: run due async work now
 * - `POST /__aleph/seed` `{ Entity: rows, users?: { id: profile } }`
 * - `POST /__aleph/reset`: drop all data
 */
export async function startDevServer(
  initialManifest: string,
  options: DevServerOptions = {},
): Promise<Server> {
  let manifestJson = initialManifest;
  let validation = validate(manifestJson);
  let engine = new DevEngine(manifestJson, { seed: Date.now() });
  const log = options.log ?? (() => {});
  const activity: Activity[] = [];
  let seq = 0;
  const record = (event: ActivityEvent) => {
    activity.push({ ...event, seq: ++seq, at: Date.now() } as Activity);
    if (activity.length > MAX_ACTIVITY) activity.splice(0, activity.length - MAX_ACTIVITY);
  };
  const mocks = [...(options.mocks ?? [])];

  function configure(target: DevEngine): void {
    for (const mock of mocks) {
      target.mock(
        mock.integration,
        mock.operation,
        mock.error === undefined ? mock.response : { error: mock.error },
      );
    }
    for (const [name, value] of Object.entries(options.secrets ?? {}))
      target.setSecret(name, value);
  }

  function seed(data: SeedData): number {
    let inserted = 0;
    for (const [entity, rows] of Object.entries(data)) {
      if (entity === "users") {
        for (const [id, profile] of Object.entries(
          rows as Record<string, Record<string, unknown>>,
        )) {
          engine.putUser(id, profile);
        }
        continue;
      }
      inserted += engine.seed(entity, rows as Record<string, unknown>[]);
    }
    return inserted;
  }

  function persist(): void {
    if (!options.dataFile) return;
    mkdirSync(dirname(options.dataFile), { recursive: true });
    writeFileSync(options.dataFile, engine.snapshot());
  }

  configure(engine);
  if (options.dataFile && existsSync(options.dataFile)) {
    engine.restore(readFileSync(options.dataFile, "utf8"));
    log(`  restored data from ${options.dataFile}`);
  }
  if (options.seed) {
    const inserted = seed(options.seed);
    log(`  seeded ${inserted} row${inserted === 1 ? "" : "s"}`);
    persist();
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
    if (report.ran > 0 || report.fired > 0) persist();
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
        mocks.push(mock);
        const response = mock.error === undefined ? mock.response : { error: mock.error };
        engine.mock(mock.integration, mock.operation, response);
        return { ok: true };
      }
      case "POST /__aleph/seed": {
        const inserted = seed(body as SeedData);
        persist();
        return { inserted };
      }
      case "POST /__aleph/reset":
        engine.restore("[]");
        persist();
        return { ok: true };
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
      .then((raw) => {
        let body: unknown;
        try {
          body = raw.trim() === "" ? undefined : (JSON.parse(raw) as unknown);
        } catch {
          send(res, 400, { error: "invalid_json", message: "the body isn't valid JSON" });
          return;
        }
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
        const headers: Record<string, string> = {};
        for (const [name, value] of Object.entries(req.headers)) {
          if (value === undefined || SKIPPED_HEADERS.has(name)) continue;
          headers[name] = Array.isArray(value) ? value.join(", ") : value;
        }
        const request: Request = {
          method: method as Method,
          path: url.pathname,
          query: Object.fromEntries(url.searchParams),
          headers,
        };
        const caller = callerFromHeaders(req);
        if (caller) request.caller = caller;
        if (body !== undefined) {
          request.body = body;
          request.rawBody = raw;
        }
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
          attempts: response.attempts,
          ms,
          body: response.body,
          logs: response.logs ?? [],
        });
        processDue();
        if (response.usage.writes > 0) persist();
        const usage = response.usage;
        send(res, response.status, response.body, {
          ...response.headers,
          ...(response.cookies?.length ? { "set-cookie": response.cookies } : {}),
          "x-aleph-usage": JSON.stringify(usage),
          ...(response.route ? { "x-aleph-route": response.route } : {}),
          ...(response.replayed ? { "x-aleph-replayed": "true" } : {}),
        });
        const conflicts =
          response.attempts > 1 ? ` (hot: ${response.attempts - 1} conflict retries)` : "";
        log(
          `${method} ${url.pathname} → ${response.status} ${response.route ?? "-"} ` +
            `(reads ${usage.reads}, writes ${usage.writes}, ops ${usage.ops}) ${ms}ms${conflicts}`,
        );
        for (const entry of response.logs ?? []) {
          log(`  ↳ log ${entry.label}: ${JSON.stringify(entry.value)}`);
        }
      })
      .catch((error: unknown) =>
        send(res, 500, {
          error: "dev_server_error",
          message: error instanceof Error ? error.message : String(error),
        }),
      );
  });

  if (options.watch) {
    const { file, reload } = options.watch;
    let pending: ReturnType<typeof setTimeout> | undefined;
    const watcher = watch(dirname(file), { recursive: true }, (_, name) => {
      if (!name || !/\.(ts|tsx|js|mjs|json)$/.test(name) || name.includes("node_modules")) return;
      if (options.dataFile && name.endsWith(options.dataFile.split("/").at(-1) ?? "")) return;
      clearTimeout(pending);
      pending = setTimeout(() => {
        reload()
          .then((next) => {
            const snapshot = engine.snapshot();
            const replacement = new DevEngine(next, { seed: Date.now() });
            configure(replacement);
            replacement.restore(snapshot);
            engine.dispose();
            engine = replacement;
            manifestJson = next;
            validation = validate(next);
            record({ kind: "reload", ok: true, message: `reloaded ${name}` });
            log(`↻ reloaded after ${name} changed (data kept)`);
          })
          .catch((error: unknown) => {
            const message = error instanceof Error ? error.message : String(error);
            record({ kind: "reload", ok: false, message });
            log(`✖ reload failed, still serving the previous version:\n${message}`);
          });
      }, 100);
    });
    server.on("close", () => watcher.close());
  }

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
