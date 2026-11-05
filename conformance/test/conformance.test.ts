/**
 * The conformance suite, TypeScript side:
 * 1. Building each case's `app.ts` with the SDK must produce its golden `manifest.json`.
 * 2. The core (through WASM) must accept it and compute the golden `bounds.json`.
 * 3. Every scenario step must produce the expected response, and every response's actual usage
 *    must fit within its route's static bounds.
 *
 * The Rust side (`crates/aleph-core/tests/conformance.rs`) runs the same files natively.
 * `pnpm --filter @aleph/conformance update` rewrites the golden files.
 */
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { DevEngine, validate } from "@aleph/core";
import type { CheckReport, Cost, Request, Response } from "@aleph/manifest";
import type { App } from "@aleph/sdk";
import { describe, expect, it } from "vitest";
import { type Scenario, subsetMismatch, substitute, valueAt } from "./scenario.js";

const casesDir = join(import.meta.dirname, "..", "cases");
const invalidDir = join(import.meta.dirname, "..", "invalid");
const update = process.env["UPDATE_GOLDEN"] === "1";
const cases = readdirSync(casesDir, { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .map((d) => d.name);

function golden(path: string, actual: string): void {
  if (update) {
    writeFileSync(path, actual);
    return;
  }
  expect(actual, `${path} is stale; run \`pnpm --filter @aleph/conformance update\``).toBe(
    readFileSync(path, "utf8"),
  );
}

function fits(usage: Cost, bounds: Cost): boolean {
  return (Object.keys(bounds) as (keyof Cost)[]).every((k) => usage[k] <= bounds[k]);
}

describe.each(cases)("case %s", (name) => {
  const dir = join(casesDir, name);

  it("builds to the golden manifest and bounds", async () => {
    const { default: app } = (await import(join(dir, "app.ts"))) as { default: App };
    const json = app.toJSON();
    golden(join(dir, "manifest.json"), json);

    const result = validate(json);
    if (!result.ok) throw new Error(JSON.stringify(result.diagnostics, null, 2));
    golden(join(dir, "bounds.json"), `${JSON.stringify(result.report, null, 2)}\n`);
  });

  it("runs every scenario step as expected, within bounds", () => {
    const manifest = readFileSync(join(dir, "manifest.json"), "utf8");
    const report = JSON.parse(readFileSync(join(dir, "bounds.json"), "utf8")) as CheckReport;
    const scenario = JSON.parse(readFileSync(join(dir, "scenarios.json"), "utf8")) as Scenario;
    const engine = new DevEngine(manifest, { seed: scenario.seed });
    const vars: Record<string, string> = {};

    scenario.steps.forEach((step, i) => {
      const request = substitute(step.request, vars) as Request;
      const response: Response = engine.handle(request, scenario.startMs + i * scenario.stepMs);
      const where = `step ${i} "${step.name}"`;
      expect(response.status, `${where}: ${JSON.stringify(response.body)}`).toBe(
        step.expect.status,
      );
      if (step.expect.body !== undefined) {
        const mismatch = subsetMismatch(step.expect.body, response.body);
        expect(mismatch, `${where}: ${JSON.stringify(response.body)}`).toBeUndefined();
      }
      if (step.expect.replayed !== undefined) {
        expect(response.replayed ?? false, `${where}: replayed`).toBe(step.expect.replayed);
      }
      if (response.route !== undefined) {
        const bounds = report.routes[response.route]?.bounds;
        expect(bounds, `${where}: no bounds for ${response.route}`).toBeDefined();
        if (bounds) expect(fits(response.usage, bounds), `${where}: usage over bounds`).toBe(true);
      }
      for (const [variable, path] of Object.entries(step.capture ?? {})) {
        const value = valueAt(response, path);
        expect(typeof value, `${where}: capture ${path}`).toBe("string");
        vars[variable] = String(value);
      }
    });
    engine.dispose();
  });
});

describe("invalid manifests", () => {
  const files = readdirSync(invalidDir).filter((f) => f.endsWith(".json"));

  it.each(files)("%s is rejected with the expected diagnostics", (file) => {
    const { expect: codes, manifest } = JSON.parse(
      readFileSync(join(invalidDir, file), "utf8"),
    ) as {
      expect: string[];
      manifest: unknown;
    };
    const result = validate(JSON.stringify(manifest));
    expect(result.ok).toBe(false);
    if (!result.ok)
      expect([...new Set(result.diagnostics.map((d) => d.code))].sort()).toEqual([...codes].sort());
  });
});
