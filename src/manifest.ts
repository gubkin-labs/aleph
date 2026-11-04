import { z } from "zod";

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue =
  | JsonPrimitive
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

export type SchemaNode =
  | {
      type: "string";
      format?: "email" | "uuid" | undefined;
      maxLength?: number | undefined;
      minLength?: number | undefined;
      optional?: boolean | undefined;
      uniqueWithin?: string | undefined;
    }
  | {
      type: "number";
      max?: number | undefined;
      min?: number | undefined;
      optional?: boolean | undefined;
    }
  | { type: "boolean"; optional?: boolean | undefined }
  | {
      type: "decimal";
      precision: number;
      scale: number;
      optional?: boolean | undefined;
    }
  | {
      type: "enum";
      values: readonly string[];
      optional?: boolean | undefined;
    }
  | {
      type: "object";
      properties: Readonly<Record<string, SchemaNode>>;
      optional?: boolean | undefined;
    }
  | {
      type: "array";
      items: SchemaNode;
      optional?: boolean | undefined;
    };

export type ExpressionNode =
  | { kind: "literal"; value: JsonValue }
  | { kind: "argument"; path: readonly string[] }
  | { kind: "binding"; name: string }
  | {
      kind: "binary";
      operator: "add" | "multiply" | "eq";
      left: ExpressionNode;
      right: ExpressionNode;
    }
  | { kind: "object"; properties: Readonly<Record<string, ExpressionNode>> }
  | { kind: "array"; items: readonly ExpressionNode[] };

export type OperationStep =
  | { kind: "derive"; name: string; expression: ExpressionNode }
  | {
      kind: "record.put";
      name: string;
      collection: string;
      recordId: ExpressionNode;
      value: ExpressionNode;
    }
  | {
      kind: "record.get";
      name: string;
      collection: string;
      recordId: ExpressionNode;
    }
  | {
      kind: "record.list";
      name: string;
      collection: string;
      limit: number;
    }
  | { kind: "return"; expression: ExpressionNode };

export interface ManifestOperation {
  id: string;
  input: SchemaNode;
  output: SchemaNode;
  steps: readonly OperationStep[];
}

export interface ManifestRoute {
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE";
  path: string;
  operationId: string;
  authProfileId?: string | undefined;
}

export interface ManifestV1 {
  formatVersion: 1;
  appId: string;
  operations: readonly ManifestOperation[];
  routes: readonly ManifestRoute[];
}

const jsonValueSchema: z.ZodType<JsonValue> = z.lazy(() =>
  z.union([
    z.string(),
    z.number().finite(),
    z.boolean(),
    z.null(),
    z.array(jsonValueSchema),
    z.record(z.string(), jsonValueSchema),
  ])
);

const schemaNodeSchema: z.ZodType<SchemaNode> = z.lazy(() =>
  z.discriminatedUnion("type", [
    z
      .object({
        format: z.enum(["email", "uuid"]).optional(),
        maxLength: z.number().int().positive().optional(),
        minLength: z.number().int().nonnegative().optional(),
        optional: z.boolean().optional(),
        type: z.literal("string"),
        uniqueWithin: z.string().min(1).optional(),
      })
      .strict(),
    z
      .object({
        max: z.number().finite().optional(),
        min: z.number().finite().optional(),
        optional: z.boolean().optional(),
        type: z.literal("number"),
      })
      .strict(),
    z
      .object({
        optional: z.boolean().optional(),
        type: z.literal("boolean"),
      })
      .strict(),
    z
      .object({
        optional: z.boolean().optional(),
        precision: z.number().int().positive(),
        scale: z.number().int().nonnegative(),
        type: z.literal("decimal"),
      })
      .strict(),
    z
      .object({
        optional: z.boolean().optional(),
        type: z.literal("enum"),
        values: z.array(z.string()).min(1),
      })
      .strict(),
    z
      .object({
        optional: z.boolean().optional(),
        properties: z.record(z.string(), schemaNodeSchema),
        type: z.literal("object"),
      })
      .strict(),
    z
      .object({
        items: schemaNodeSchema,
        optional: z.boolean().optional(),
        type: z.literal("array"),
      })
      .strict(),
  ])
);

const expressionNodeSchema: z.ZodType<ExpressionNode> = z.lazy(() =>
  z.discriminatedUnion("kind", [
    z
      .object({
        kind: z.literal("literal"),
        value: jsonValueSchema,
      })
      .strict(),
    z
      .object({
        kind: z.literal("argument"),
        path: z.array(z.string().min(1)).min(1),
      })
      .strict(),
    z
      .object({
        kind: z.literal("binding"),
        name: z.string().min(1),
      })
      .strict(),
    z
      .object({
        kind: z.literal("binary"),
        left: expressionNodeSchema,
        operator: z.enum(["add", "multiply", "eq"]),
        right: expressionNodeSchema,
      })
      .strict(),
    z
      .object({
        kind: z.literal("object"),
        properties: z.record(z.string(), expressionNodeSchema),
      })
      .strict(),
    z
      .object({
        items: z.array(expressionNodeSchema),
        kind: z.literal("array"),
      })
      .strict(),
  ])
);

const operationStepSchema: z.ZodType<OperationStep> = z.discriminatedUnion(
  "kind",
  [
    z
      .object({
        expression: expressionNodeSchema,
        kind: z.literal("derive"),
        name: z.string().min(1),
      })
      .strict(),
    z
      .object({
        collection: z.string().min(1),
        kind: z.literal("record.put"),
        name: z.string().min(1),
        recordId: expressionNodeSchema,
        value: expressionNodeSchema,
      })
      .strict(),
    z
      .object({
        collection: z.string().min(1),
        kind: z.literal("record.get"),
        name: z.string().min(1),
        recordId: expressionNodeSchema,
      })
      .strict(),
    z
      .object({
        collection: z.string().min(1),
        kind: z.literal("record.list"),
        limit: z.number().int().min(1).max(1000),
        name: z.string().min(1),
      })
      .strict(),
    z
      .object({
        expression: expressionNodeSchema,
        kind: z.literal("return"),
      })
      .strict(),
  ]
);

const operationSchema: z.ZodType<ManifestOperation> = z
  .object({
    id: z.string().min(1),
    input: schemaNodeSchema,
    output: schemaNodeSchema,
    steps: z.array(operationStepSchema).min(1),
  })
  .strict();

const routeSchema: z.ZodType<ManifestRoute> = z
  .object({
    authProfileId: z.string().min(1).optional(),
    method: z.enum(["GET", "POST", "PUT", "PATCH", "DELETE"]),
    operationId: z.string().min(1),
    path: z.string().startsWith("/"),
  })
  .strict();

export const manifestSchema: z.ZodType<ManifestV1> = z
  .object({
    appId: z.string().min(1),
    formatVersion: z.literal(1),
    operations: z.array(operationSchema),
    routes: z.array(routeSchema),
  })
  .strict();

export class ManifestValidationError extends Error {
  readonly issues: readonly string[];

  constructor(issues: readonly string[]) {
    super(`Invalid Aleph manifest: ${issues.join("; ")}`);
    this.name = "ManifestValidationError";
    this.issues = issues;
  }
}

export const validateManifest = (input: unknown): ManifestV1 => {
  const parsed = manifestSchema.safeParse(input);

  if (!parsed.success) {
    const issues = parsed.error.issues.map((issue) => {
      const path = issue.path.map(String).join(".");
      return `${path.length > 0 ? path : "manifest"}: ${issue.message}`;
    });

    throw new ManifestValidationError(issues);
  }

  const operationIds = new Set<string>();
  for (const operation of parsed.data.operations) {
    if (operationIds.has(operation.id)) {
      throw new ManifestValidationError([
        `operations: duplicate operation id '${operation.id}'`,
      ]);
    }

    operationIds.add(operation.id);

    const returnSteps = operation.steps.filter((step) => step.kind === "return");
    if (
      returnSteps.length !== 1 ||
      operation.steps.at(-1)?.kind !== "return"
    ) {
      throw new ManifestValidationError([
        `operations.${operation.id}.steps: expected exactly one final return step`,
      ]);
    }

    const bindings = new Set<string>();
    for (const step of operation.steps) {
      if (step.kind === "return") {
        continue;
      }

      if (bindings.has(step.name)) {
        throw new ManifestValidationError([
          `operations.${operation.id}.steps: duplicate binding '${step.name}'`,
        ]);
      }

      bindings.add(step.name);
    }
  }

  const routeKeys = new Set<string>();
  for (const route of parsed.data.routes) {
    if (!operationIds.has(route.operationId)) {
      throw new ManifestValidationError([
        `routes: operation '${route.operationId}' does not exist`,
      ]);
    }

    const routeKey = `${route.method} ${route.path}`;
    if (routeKeys.has(routeKey)) {
      throw new ManifestValidationError([`routes: duplicate route '${routeKey}'`]);
    }

    routeKeys.add(routeKey);
  }

  return parsed.data;
};

const isRecord = (
  value: unknown
): value is Readonly<Record<string, unknown>> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const sortKeys = (value: unknown): unknown => {
  if (Array.isArray(value)) {
    return value.map(sortKeys);
  }

  if (!isRecord(value)) {
    return value;
  }

  const sortedEntries: Array<readonly [string, unknown]> = [];

  for (const key of Object.keys(value).sort()) {
    const child = value[key];
    if (child !== undefined) {
      sortedEntries.push([key, sortKeys(child)]);
    }
  }

  return Object.fromEntries(sortedEntries);
};

export const serializeManifest = (input: unknown): string => {
  const manifest = validateManifest(input);
  return `${JSON.stringify(sortKeys(manifest), null, 2)}\n`;
};
