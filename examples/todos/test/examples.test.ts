import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { generateClient } from "@aleph/cli";
import { runExamples, validate } from "@aleph/core";
import { describe, expect, it } from "vitest";
import app from "../app.ts";

describe("todos", () => {
  it("passes every route example (aleph test)", () => {
    const failures = runExamples(app.toJSON()).filter((r) => r.failures.length > 0);
    expect(failures, JSON.stringify(failures, null, 2)).toEqual([]);
  });

  it("has an up-to-date generated client (pnpm client to regenerate)", () => {
    const result = validate(app.toJSON());
    if (!result.ok) throw new Error(JSON.stringify(result.diagnostics));
    const source = generateClient(app.build(), result.report);
    const file = join(import.meta.dirname, "..", "src", "client.ts");
    if (process.env["UPDATE_CLIENT"] === "1") writeFileSync(file, source);
    expect(readFileSync(file, "utf8"), "src/client.ts is stale: run `pnpm client`").toBe(source);
  });
});
