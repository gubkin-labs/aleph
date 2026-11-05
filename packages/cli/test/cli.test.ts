import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { main, startDevServer } from "../src/index.js";

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

  it("explains a module that doesn't export an app", async () => {
    const { io, err } = capture();
    expect(await main(["build", join(import.meta.dirname, "cli.test.ts")], io)).toBe(1);
    expect(err.join("\n")).toMatch(/couldn't load|must `export default` an app/);
  });
});

describe("aleph check", () => {
  it("prints the cost table for a valid app", async () => {
    const { io, out } = capture();
    expect(await main(["check", join(cases, "projects", "manifest.json")], io)).toBe(0);
    expect(out.join("\n")).toMatch(/createProject\s+3 \(1%\)\s+3 \(6%\)/);
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
});
