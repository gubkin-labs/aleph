import { describe, expect, it } from "vitest";
import {
  application,
  arg,
  ManifestValidationError,
  operation,
  ref,
  route,
  schema,
  serializeManifest,
  validateManifest,
} from "../src/index.js";

const buildScaleManifest = () => {
  const scale = operation("numbers.scale")
    .input(schema.object({ value: schema.number() }))
    .output(schema.object({ value: schema.number() }))
    .derive("scaled", arg("value").add(2).multiply(100))
    .return(ref("scaled"));

  return application("scale-demo")
    .operation(scale)
    .route(route.post("/numbers/scale").operation(scale))
    .build();
};

describe("manifest builder", () => {
  it("builds a serializable plan from typed builders", () => {
    const manifest = buildScaleManifest();
    const serialized = serializeManifest(manifest);
    const parsed: unknown = JSON.parse(serialized);
    const validated = validateManifest(parsed);

    expect(validated.appId).toBe("scale-demo");
    expect(validated.operations).toHaveLength(1);
    expect(validated.operations[0]?.steps[0]?.kind).toBe("derive");
    expect(validated.routes[0]?.operationId).toBe("numbers.scale");
  });

  it("serializes objects with stable key ordering", () => {
    const first = serializeManifest(buildScaleManifest());
    const second = serializeManifest(buildScaleManifest());

    expect(first).toBe(second);
    expect(first.endsWith("\n")).toBe(true);
  });

  it("rejects routes that refer to missing operations", () => {
    expect(() =>
      application("broken")
        .route(route.get("/missing").operation("missing.operation"))
        .build()
    ).toThrow(ManifestValidationError);
  });

  it("rejects duplicate operation identifiers", () => {
    const first = operation("same.operation")
      .input(schema.object({}))
      .output(schema.object({}))
      .return({ ok: true });
    const second = operation("same.operation")
      .input(schema.object({}))
      .output(schema.object({}))
      .return({ ok: true });

    expect(() =>
      application("duplicate")
        .operation(first)
        .operation(second)
        .build()
    ).toThrow(/duplicate operation id/);
  });
});
