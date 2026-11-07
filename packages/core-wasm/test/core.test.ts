import type { Manifest } from "@aleph-lang/manifest";
import { describe, expect, it } from "vitest";
import {
  DevEngine,
  defaultLimits,
  formatVersion,
  InvalidManifestError,
  validate,
} from "../src/index.js";

const textField = { type: { kind: "string" as const, min: 1, max: 100 } };

const manifest: Manifest = {
  formatVersion: "0.2",
  app: { name: "notes" },
  entities: {
    Note: {
      key: ["noteId"],
      fields: {
        noteId: { type: { kind: "id" }, default: { kind: "newId" } },
        text: textField,
      },
      indexes: {},
    },
  },
  counters: {},
  routes: [
    {
      id: "createNote",
      method: "POST",
      path: "/notes",
      auth: { kind: "public" },
      input: { params: {}, query: {}, body: { text: textField } },
      errors: {},
      plan: {
        body: [
          {
            kind: "let",
            var: 0,
            op: {
              op: "insert",
              entity: "Note",
              values: { text: { kind: "input", source: "body", name: "text" } },
            },
          },
        ],
        output: { kind: "var", id: 0 },
      },
    },
  ],
};

describe("@aleph-lang/core", () => {
  it("reports the format version and limits", () => {
    expect(formatVersion()).toBe("0.2");
    expect(defaultLimits().writes).toBe(50);
  });

  it("validates and reports worst-case bounds", () => {
    const result = validate(manifest);
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.report.routes["createNote"]?.bounds).toMatchObject({ reads: 1, writes: 1 });
    }
  });

  it("returns every diagnostic for an invalid manifest", () => {
    const broken = structuredClone(manifest);
    const route = broken.routes[0];
    if (!route) throw new Error("fixture has a route");
    route.plan.output = { kind: "var", id: 7 };
    route.path = "/notes/:id";
    const result = validate(broken);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.diagnostics.map((d) => d.code).sort()).toEqual([
        "path_param_mismatch",
        "undefined_var",
      ]);
    }
  });

  it("runs requests in the dev engine, deterministically", () => {
    const run = () => {
      const engine = new DevEngine(manifest, { seed: 42 });
      const created = engine.handle(
        { method: "POST", path: "/notes", body: { text: "hi" } },
        1_000,
      );
      const invalid = engine.handle({ method: "POST", path: "/notes", body: { text: "" } }, 1_000);
      engine.dispose();
      return { created, invalid };
    };
    const { created, invalid } = run();
    expect(created.status).toBe(200);
    expect(created.body).toMatchObject({ text: "hi" });
    expect(created.usage).toMatchObject({ reads: 1, writes: 1 });
    expect(invalid.status).toBe(400);
    expect(invalid.body).toMatchObject({ error: "invalid_input" });
    expect(run().created.body).toEqual(created.body);
  });

  it("throws typed diagnostics for invalid manifests", () => {
    expect(() => new DevEngine({ ...manifest, formatVersion: "9" })).toThrow(InvalidManifestError);
  });
});
