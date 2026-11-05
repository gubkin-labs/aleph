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
import { DevEngine } from "@aleph/core";
import type { Caller, Method, Request } from "@aleph/manifest";

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

/** Options for {@link startDevServer}. */
export interface DevServerOptions {
  port?: number;
  host?: string;
  /** Called once per request with a one-line summary. */
  log?: (line: string) => void;
}

/** Starts the dev server. Resolves once it's listening. */
export async function startDevServer(
  manifestJson: string,
  options: DevServerOptions = {},
): Promise<Server> {
  const engine = new DevEngine(manifestJson, { seed: Date.now() });
  const log = options.log ?? (() => {});
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
        const usage = response.usage;
        send(res, response.status, response.body, {
          "x-aleph-usage": JSON.stringify(usage),
          ...(response.route ? { "x-aleph-route": response.route } : {}),
          ...(response.replayed ? { "x-aleph-replayed": "true" } : {}),
        });
        const ms = (performance.now() - started).toFixed(1);
        log(
          `${method} ${url.pathname} → ${response.status} ${response.route ?? "-"} ` +
            `(reads ${usage.reads}, writes ${usage.writes}, ops ${usage.ops}) ${ms}ms`,
        );
      })
      .catch(() => send(res, 400, { error: "invalid_json", message: "the body isn't valid JSON" }));
  });
  await new Promise<void>((resolve) =>
    server.listen(options.port ?? 3000, options.host ?? "127.0.0.1", resolve),
  );
  server.on("close", () => engine.dispose());
  return server;
}
