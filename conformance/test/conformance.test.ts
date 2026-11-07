/**
 * The conformance suite, TypeScript side:
 * 1. Building each case's `app.ts` with the SDK must produce its golden `manifest.json`.
 * 2. The core (through WASM) must accept it and compute the golden `bounds.json`.
 * 3. Every scenario step must produce the expected response, and every response's actual usage
 *    must fit within its route's static bounds.
 *
 * The Rust side (`crates/aleph-core/tests/conformance.rs`) runs the same files natively.
 * `pnpm --filter @aleph-lang/conformance update` rewrites the golden files.
 */
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { compareManifests, DevEngine, runExamples, validate } from "@aleph-lang/core";
import type { CheckReport, Cost, Request, Response } from "@aleph-lang/manifest";
import type { App } from "@aleph-lang/sdk";
import { describe, expect, it } from "vitest";
import {
  type Scenario,
  subsetMismatch,
  substitute,
  unorderedMismatch,
  valueAt,
} from "./scenario.js";

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
  expect(actual, `${path} is stale; run \`pnpm --filter @aleph-lang/conformance update\``).toBe(
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

  it("passes every route example", () => {
    const manifest = readFileSync(join(dir, "manifest.json"), "utf8");
    const failures = runExamples(manifest).filter((r) => r.failures.length > 0);
    expect(failures, JSON.stringify(failures, null, 2)).toEqual([]);
  });

  it("runs every scenario step as expected, within bounds", () => {
    const manifest = readFileSync(join(dir, "manifest.json"), "utf8");
    const report = JSON.parse(readFileSync(join(dir, "bounds.json"), "utf8")) as CheckReport;
    const staticReport = report;
    const scenario = JSON.parse(readFileSync(join(dir, "scenarios.json"), "utf8")) as Scenario;
    const engine = new DevEngine(manifest, { seed: scenario.seed });
    const vars: Record<string, string> = {};

    let extra = 0;
    scenario.steps.forEach((step, i) => {
      extra += step.advanceMs ?? 0;
      const now = scenario.startMs + i * scenario.stepMs + extra;
      const where = `step ${i} "${step.name}"`;
      if (step.secret) {
        engine.setSecret(step.secret.name, step.secret.value);
        return;
      }
      if (step.user) {
        engine.putUser(step.user.id, step.user.profile);
        return;
      }
      if (step.seed) {
        engine.seed(step.seed.entity, substitute(step.seed.rows, vars) as never, now);
        return;
      }
      if (step.mock) {
        const { integration, operation, response, error } = step.mock;
        engine.mock(integration, operation, error === undefined ? response : { error });
        return;
      }
      if (step.process) {
        const report = engine.process(now);
        const boundsReport = staticReport;
        const expected = step.expect ?? {};
        const checks: [string, string | undefined][] = [
          [
            "report",
            expected.report === undefined
              ? undefined
              : subsetMismatch(expected.report, report, "report"),
          ],
          [
            "deliveries",
            expected.deliveries === undefined
              ? undefined
              : unorderedMismatch(
                  substitute(expected.deliveries, vars) as unknown[],
                  engine.takeDeliveries(),
                  "deliveries",
                ),
          ],
          [
            "calls",
            expected.calls === undefined
              ? undefined
              : unorderedMismatch(
                  substitute(expected.calls, vars) as unknown[],
                  engine.takeCalls(),
                  "calls",
                ),
          ],
        ];
        for (const run of report.runs) {
          const bounds = boundsReport.handlers?.[run.handler]?.bounds;
          expect(
            bounds && fits(run.usage, bounds),
            `${where}: ${run.handler} usage over bounds`,
          ).toBe(true);
        }
        for (const [what, mismatch] of checks) {
          expect(mismatch, `${where} ${what}: ${JSON.stringify(report)}`).toBeUndefined();
        }
        return;
      }
      const request = substitute(step.request, vars) as Request;
      const response: Response = engine.handle(request, now);
      const expectation = step.expect ?? {};
      if (expectation.status !== undefined) {
        expect(response.status, `${where}: ${JSON.stringify(response.body)}`).toBe(
          expectation.status,
        );
      }
      if (expectation.body !== undefined) {
        const mismatch = subsetMismatch(substitute(expectation.body, vars), response.body);
        expect(mismatch, `${where}: ${JSON.stringify(response.body)}`).toBeUndefined();
      }
      if (expectation.replayed !== undefined) {
        expect(response.replayed ?? false, `${where}: replayed`).toBe(expectation.replayed);
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

interface Edit {
  set?: string[];
  delete?: string[];
  value?: unknown;
}

/** Applies `{ set: path, value }` and `{ delete: path }` edits to a JSON value (in place). */
function applyEdits(target: unknown, edits: Edit[] = []): void {
  for (const edit of edits) {
    const path = edit.set ?? edit.delete ?? [];
    let node = target as Record<string, unknown>;
    for (const key of path.slice(0, -1)) node = node[key] as Record<string, unknown>;
    const last = path.at(-1) as string;
    if (edit.set) node[last] = structuredClone(edit.value);
    else delete node[last];
  }
}

describe("schema evolution", () => {
  const spec = JSON.parse(
    readFileSync(join(import.meta.dirname, "..", "evolution.json"), "utf8"),
  ) as {
    base: string;
    cases: {
      name: string;
      base_edits?: Edit[];
      edits: Edit[];
      deployable: boolean;
      expect: Record<string, string>;
      migration?: unknown[];
    }[];
  };
  const base = readFileSync(join(import.meta.dirname, "..", spec.base), "utf8");

  it.each(spec.cases)("$name", (c) => {
    const deployed = JSON.parse(base) as unknown;
    applyEdits(deployed, c.base_edits);
    const next = structuredClone(deployed);
    applyEdits(next, c.edits);
    const evolution = compareManifests(JSON.stringify(deployed), JSON.stringify(next));
    expect(evolution.deployable, JSON.stringify(evolution, null, 2)).toBe(c.deployable);
    const kinds = Object.fromEntries(evolution.changes.map((ch) => [ch.path, ch.kind]));
    expect(kinds).toEqual(c.expect);
    if (c.migration) expect(evolution.migration).toEqual(c.migration);
  });
});
