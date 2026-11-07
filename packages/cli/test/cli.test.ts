import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import type { Server } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { main, startDevServer } from "../src/index.js";
import { loadFailure } from "../src/load.js";

const cases = join(import.meta.dirname, "..", "..", "..", "conformance", "cases");
const invalid = join(import.meta.dirname, "..", "..", "..", "conformance", "invalid");

function capture() {
  const out: string[] = [];
  const err: string[] = [];
  return { io: { out: (t: string) => out.push(t), err: (t: string) => err.push(t) }, out, err };
}

describe("aleph build", () => {
  it.each(["projects", "ledger"])("reproduces the golden %s manifest", async (name) => {
    const dir = mkdtempSync(join(tmpdir(), "aleph-cli-"));
    const target = join(dir, "manifest.json");
    const { io } = capture();
    expect(await main(["build", join(cases, name, "app.ts"), "-o", target], io)).toBe(0);
    expect(readFileSync(target, "utf8")).toBe(
      readFileSync(join(cases, name, "manifest.json"), "utf8"),
    );
  });

  it("explains an app that Node loads as CommonJS", () => {
    // Node's real error for an ES module in a "type": "commonjs" project. (Vitest transforms
    // imports itself, so the CLI has to be checked against Node's message directly.)
    const dir = mkdtempSync(join(tmpdir(), "aleph-cli-"));
    writeFileSync(join(dir, "package.json"), '{ "type": "commonjs" }\n');
    writeFileSync(join(dir, "app.ts"), 'import { app } from "@aleph/sdk";\nexport default app;\n');
    const node = spawnSync(process.execPath, [join(dir, "app.ts")], { encoding: "utf8" });
    const message = node.stderr.split("\n").find((l) => l.startsWith("SyntaxError")) ?? "";
    expect(message).not.toBe("");
    expect(
      loadFailure("app.ts", new SyntaxError(message.replace("SyntaxError: ", ""))).message,
    ).toMatch(/Aleph apps are ES modules[\s\S]*npm pkg set type=module/);
  });

  it("explains a module that doesn't export an app", async () => {
    const { io, err } = capture();
    expect(await main(["build", join(import.meta.dirname, "cli.test.ts")], io)).toBe(1);
    expect(err.join("\n")).toMatch(/couldn't load|must `export default` an app/);
  });
});

describe("aleph check", () => {
  it("prints each route's worst-case usage against the limits", async () => {
    const { io, out } = capture();
    expect(await main(["check", join(cases, "projects", "manifest.json")], io)).toBe(0);
    // 4 reads: billing, counter, the row's existence, and the unique slug probe.
    const text = out.join("\n");
    expect(text).toMatch(/^route\s+reads\s+writes/m);
    expect(text).toMatch(/^limit\s+250\s+50\s+2,000\s+2 MB/m);
    expect(text).toMatch(/createProject\s+4\s+2%\s+5\s+10%/);
    // No usage shows as "–", and a little as "<1%", never "0%".
    expect(text).toMatch(/projectBySlug\s+1\s+<1%\s+0\s+–/);
    expect(text).not.toMatch(/\b0%/);
  });

  it("prints every diagnostic and exits 1 for an invalid manifest", async () => {
    const { manifest } = JSON.parse(readFileSync(join(invalid, "auth-rules.json"), "utf8")) as {
      manifest: unknown;
    };
    const dir = mkdtempSync(join(tmpdir(), "aleph-cli-"));
    const file = join(dir, "bad.json");
    await import("node:fs/promises").then((fs) => fs.writeFile(file, JSON.stringify(manifest)));
    const { io, err } = capture();
    expect(await main(["check", file], io)).toBe(1);
    expect(err.join("\n")).toMatch(/unknown_role at routes\[0\]\.auth[\s\S]*3 problems/);
  });

  it("shows usage for bad invocations", async () => {
    const { io, err } = capture();
    expect(await main(["check"], io)).toBe(2);
    expect(err.join("\n")).toMatch(/needs a file/);
  });
});

describe("aleph check with source maps", () => {
  it("points diagnostics at the line in the app", async () => {
    const { io, err } = capture();
    expect(await main(["check", join(import.meta.dirname, "fixtures", "over-budget.ts")], io)).toBe(
      1,
    );
    expect(err.join("\n")).toMatch(/✖ over_budget at test\/fixtures\/over-budget\.ts:\d+:\d+/);
  });

  it("prints handler bounds against the job limits", async () => {
    const { io, out } = capture();
    expect(await main(["check", join(cases, "pipeline", "app.ts")], io)).toBe(0);
    expect(out.join("\n")).toMatch(
      /handler\s+reads[\s\S]*limit\s+500\s+200\s+5,000[\s\S]*processDoc\s+3\s+<1%/,
    );
  });
});

describe("aleph openapi / mcp", () => {
  it("describes routes as OpenAPI 3.1", async () => {
    const { io, out } = capture();
    expect(await main(["openapi", join(cases, "pipeline", "manifest.json")], io)).toBe(0);
    const doc = JSON.parse(out.join("")) as {
      openapi: string;
      paths: Record<
        string,
        Record<string, { operationId: string; responses: Record<string, unknown> }>
      >;
    };
    expect(doc.openapi).toBe("3.1.0");
    const upload = doc.paths["/orgs/{orgId}/docs"]?.["post"];
    expect(upload?.operationId).toBe("uploadDoc");
    expect(Object.keys(upload?.responses ?? {})).toEqual(
      expect.arrayContaining(["200", "401", "403", "429"]),
    );
  });

  it("lists MCP tools with JSON Schema inputs", async () => {
    const { io, out } = capture();
    expect(await main(["mcp", join(cases, "pipeline", "manifest.json")], io)).toBe(0);
    expect(JSON.parse(out.join(""))).toEqual({
      tools: [
        expect.objectContaining({
          name: "upload_doc",
          description: "Upload a document for processing",
          inputSchema: expect.objectContaining({ required: ["orgId", "title"] }),
        }),
      ],
    });
  });
});

describe("aleph dev", () => {
  it("serves the app over HTTP with dev auth headers", async () => {
    const manifest = readFileSync(join(cases, "projects", "manifest.json"), "utf8");
    const lines: string[] = [];
    const server = await startDevServer(manifest, { port: 0, log: (l) => lines.push(l) });
    try {
      const address = server.address();
      if (address === null || typeof address === "string") throw new Error("no port");
      const base = `http://127.0.0.1:${address.port}`;
      const owner = {
        "x-aleph-user": "u1",
        "x-aleph-orgs": "acme=owner",
        "content-type": "application/json",
      };

      const created = await fetch(`${base}/orgs/acme/projects`, {
        method: "POST",
        headers: owner,
        body: JSON.stringify({ name: "Hello" }),
      });
      expect(created.status).toBe(200);
      expect(await created.json()).toMatchObject({ name: "Hello", status: "active" });
      expect(created.headers.get("x-aleph-route")).toBe("createProject");

      const anonymous = await fetch(`${base}/orgs/acme/projects`);
      expect(anonymous.status).toBe(401);

      const viaKey = await fetch(`${base}/orgs/acme/projects`, {
        headers: { "x-aleph-api-key": "acme", "x-aleph-api-key-scopes": "projects:read" },
      });
      expect(await viaKey.json()).toMatchObject({ items: [{ name: "Hello" }] });

      const badJson = await fetch(`${base}/orgs/acme/projects`, {
        method: "POST",
        headers: owner,
        body: "{",
      });
      expect(badJson.status).toBe(400);
      expect(lines[0]).toMatch(
        /^POST \/orgs\/acme\/projects → 200 createProject \(reads 3, writes 3/,
      );
    } finally {
      await new Promise((resolve) => server.close(resolve));
    }
  });

  it("runs async work after requests and exposes the inspection API", async () => {
    const manifest = readFileSync(join(cases, "pipeline", "manifest.json"), "utf8");
    const lines: string[] = [];
    const server = await startDevServer(manifest, { port: 0, log: (l) => lines.push(l) });
    try {
      const address = server.address();
      if (address === null || typeof address === "string") throw new Error("no port");
      const base = `http://127.0.0.1:${address.port}`;
      const upload = await fetch(`${base}/orgs/acme/docs`, {
        method: "POST",
        headers: {
          "x-aleph-user": "u1",
          "x-aleph-orgs": "acme=member",
          "content-type": "application/json",
        },
        body: JSON.stringify({ title: "Hello" }),
      });
      expect(upload.status).toBe(200);
      expect(lines.join("\n")).toMatch(/ran enqueueProcessing/);

      const { activity } = (await (await fetch(`${base}/__aleph/activity`)).json()) as {
        activity: { kind: string; route?: string; handler?: string }[];
      };
      expect(activity.map((a) => a.route ?? a.handler)).toEqual(
        expect.arrayContaining(["uploadDoc", "enqueueProcessing"]),
      );
      const mocked = await fetch(`${base}/__aleph/mock`, {
        method: "POST",
        body: JSON.stringify({
          integration: "ocr",
          operation: "extract",
          response: { pages: 1, summary: "x" },
        }),
      });
      expect(mocked.status).toBe(200);
      const report = (await (await fetch(`${base}/__aleph/report`)).json()) as { ok: boolean };
      expect(report.ok).toBe(true);
      expect((await fetch(`${base}/__aleph/nope`)).status).toBe(404);
    } finally {
      await new Promise((resolve) => server.close(resolve));
    }
  });
});

describe("aleph test", () => {
  it("runs every route example and reports each", async () => {
    const { io, out } = capture();
    expect(await main(["test", join(cases, "commerce", "manifest.json")], io)).toBe(0);
    const text = out.join("\n");
    expect(text).toMatch(/✔ post_orders: prices the items, totals exactly/);
    expect(text).toMatch(/\d+ passed, 0 failed/);
  });

  it("fails with the mismatch when an example is wrong", async () => {
    const manifest = JSON.parse(readFileSync(join(cases, "commerce", "manifest.json"), "utf8")) as {
      routes: { examples?: { expect: { status?: number } }[] }[];
    };
    const example = manifest.routes.flatMap((r) => r.examples ?? [])[0];
    if (example) example.expect.status = 418;
    const dir = mkdtempSync(join(tmpdir(), "aleph-cli-"));
    const file = join(dir, "m.json");
    writeFileSync(file, JSON.stringify(manifest));
    const { io, out, err } = capture();
    expect(await main(["test", file], io)).toBe(1);
    expect(out.join("\n")).toMatch(/status: expected 418/);
    expect(err.join("\n")).toMatch(/1 failed/);
  });
});

describe("aleph client", () => {
  it("generates a typed client from the inferred and declared response types", async () => {
    const { io, out } = capture();
    expect(await main(["client", join(cases, "commerce", "manifest.json")], io)).toBe(0);
    const source = out.join("\n");
    expect(source).toMatch(/export function createClient/);
    // Declared `returns` and declared error details are typed.
    expect(source).toMatch(
      /postOrders: \{\n\s+input: \{ items: \{ qty: number; sku: string \}\[\]/,
    );
    expect(source).toMatch(
      /error: "unknownProducts"; message: string; details: \{ missing: number \}/,
    );
    // Inferred types carry nullability and decimals.
    expect(source).toMatch(/getDeals: \{[\s\S]*cheapestBook: Decimal \| null/);
    expect(source).toMatch(/@deprecated Use GET \/deals/);
  });
});

describe("aleph check --against", () => {
  it("classifies changes and refuses breaking ones", async () => {
    const deployed = join(cases, "commerce", "manifest.json");
    const next = JSON.parse(readFileSync(deployed, "utf8")) as {
      entities: Record<string, { fields: Record<string, unknown> }>;
    };
    delete next.entities["Product"]?.fields["tags"];
    const dir = mkdtempSync(join(tmpdir(), "aleph-cli-"));
    const file = join(dir, "next.json");
    writeFileSync(file, JSON.stringify(next));
    const { io, err } = capture();
    expect(await main(["check", file, "--against", deployed], io)).toBe(1);
    expect(err.join("\n")).toMatch(/✖ entities\.Product\.fields\.tags: field tags was removed/);
    expect(err.join("\n")).toMatch(/not deployable/);

    const same = capture();
    expect(await main(["check", deployed, "--against", deployed], same.io)).toBe(0);
    expect(same.out.join("\n")).toMatch(/no changes[\s\S]*✔ deployable/);
  });
});

describe("aleph dev, language v0.3", () => {
  async function serve(options: Parameters<typeof startDevServer>[1] = {}) {
    const manifest = readFileSync(join(cases, "commerce", "manifest.json"), "utf8");
    const lines: string[] = [];
    const server = await startDevServer(manifest, {
      port: 0,
      log: (l) => lines.push(l),
      ...options,
    });
    const address = server.address();
    if (address === null || typeof address === "string") throw new Error("no port");
    return { base: `http://127.0.0.1:${address.port}`, server, lines };
  }
  const close = (server: Server) => new Promise((resolve) => server.close(resolve));

  it("passes headers, raw bodies, statuses, cookies, CORS, and logs through", async () => {
    const { base, server, lines } = await serve({
      secrets: { PAYMENTS_SECRET: "whsec_live" },
      seed: {
        Product: [{ sku: "BK-1", name: "Rust", price: "20", category: "books" }],
        users: { c1: { email: "c1@example.com", name: "Cee" } },
      },
    });
    try {
      const c1 = {
        "x-aleph-user": "c1",
        "x-aleph-profile": JSON.stringify({ email: "c1@example.com", name: "Cee" }),
        "content-type": "application/json",
      };
      const order = await fetch(`${base}/orders`, {
        method: "POST",
        headers: c1,
        body: JSON.stringify({
          items: [{ sku: "BK-1", qty: 2 }],
          shipTo: { city: "Rome", zip: "1" },
        }),
      });
      expect(order.status).toBe(201);
      const { orderId } = (await order.json()) as { orderId: string };
      expect(order.headers.get("location")).toBe(`/orders/${orderId}`);
      expect(lines.join("\n")).toMatch(/log total: "40.00"/);

      const me = await fetch(`${base}/me`, { headers: c1 });
      expect(me.headers.get("set-cookie")).toMatch(/^seen=1; Path=\/; Max-Age=3600; HttpOnly/);
      expect(await me.json()).toMatchObject({ directoryName: "Cee", greeting: "Hi, Cee" });

      const deleted = await fetch(`${base}/orders/${orderId}`, { method: "DELETE", headers: c1 });
      expect(deleted.status).toBe(204);
      expect(await deleted.text()).toBe("");

      const preflight = await fetch(`${base}/orders`, {
        method: "OPTIONS",
        headers: { origin: "https://shop.example.com" },
      });
      expect(preflight.status).toBe(204);
      expect(preflight.headers.get("access-control-allow-origin")).toBe("https://shop.example.com");

      const raw = JSON.stringify({ customerId: "c1", eventId: "e1", orderId });
      const { createHmac } = await import("node:crypto");
      const signature = createHmac("sha256", "whsec_live").update(raw).digest("hex");
      const hook = await fetch(`${base}/webhooks/payments`, {
        method: "POST",
        headers: { "x-signature": signature, "content-type": "application/json" },
        body: raw,
      });
      expect(hook.status).toBe(200);
      const forged = await fetch(`${base}/webhooks/payments`, {
        method: "POST",
        headers: { "x-signature": "00", "content-type": "application/json" },
        body: raw,
      });
      expect(forged.status).toBe(401);

      const redirect = await fetch(`${base}/go/BK-1`, { redirect: "manual" });
      expect(redirect.status).toBe(301);
      expect(redirect.headers.get("location")).toBe("/products/BK-1");
      const legacy = await fetch(`${base}/legacy/products`);
      expect(legacy.headers.get("deprecation")).toBe("true");
    } finally {
      await close(server);
    }
  });

  it("keeps data in a file across restarts", async () => {
    const dataFile = join(mkdtempSync(join(tmpdir(), "aleph-dev-")), "data.json");
    const first = await serve({ dataFile });
    await fetch(`${first.base}/products/BK-9`, {
      method: "PUT",
      headers: { "x-aleph-user": "admin", "content-type": "application/json" },
      body: JSON.stringify({ name: "Kept", price: "1", category: "tools" }),
    });
    await close(first.server);
    const second = await serve({ dataFile });
    try {
      const deals = await fetch(`${second.base}/legacy/products`);
      expect(await deals.json()).toEqual({ products: ["BK-9"] });
      expect(second.lines.join("\n")).toMatch(/restored data/);
    } finally {
      await close(second.server);
    }
  });
});
