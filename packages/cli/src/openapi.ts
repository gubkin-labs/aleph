/** OpenAPI and MCP descriptions generated from a manifest. */
import type { CheckReport, Field, FieldType, Manifest, Route, TypeSchema } from "@aleph/manifest";

type JsonSchema = Record<string, unknown>;

/** JSON Schema for a field type. */
export function fieldSchema(type: FieldType): JsonSchema {
  switch (type.kind) {
    case "string":
      return { type: "string", minLength: type.min, maxLength: type.max };
    case "int":
      return {
        type: "integer",
        ...(type.min === undefined ? {} : { minimum: type.min }),
        ...(type.max === undefined ? {} : { maximum: type.max }),
      };
    case "bool":
      return { type: "boolean" };
    case "enum":
      return { type: "string", enum: type.values };
    case "id":
      return { type: "string", pattern: "^[0-7][0-9A-HJKMNP-TV-Z]{25}$", description: "ULID" };
    case "timestamp":
      return { type: "integer", description: "milliseconds since the Unix epoch" };
    case "array":
      return { type: "array", items: fieldSchema(type.items), maxItems: type.max };
    case "object":
      return objectSchema(type.fields);
    case "decimal":
      return {
        type: "string",
        pattern: "^-?\\d+(\\.\\d+)?$",
        description: `decimal with ${type.scale} digits after the point`,
        "x-aleph-decimal": {
          scale: type.scale,
          ...(type.min ? { min: type.min } : {}),
          ...(type.max ? { max: type.max } : {}),
        },
      };
    case "timeZone":
      return { type: "string", description: "IANA time zone, like Europe/Berlin" };
  }
}

/** JSON Schema for a response value. */
export function responseSchema(schema: TypeSchema): JsonSchema {
  switch (schema.kind) {
    case "any":
      return {};
    case "null":
      return { type: "null" };
    case "bool":
      return { type: "boolean" };
    case "int":
      return { type: "integer" };
    case "decimal":
      return { type: "string", pattern: "^-?\\d+(\\.\\d+)?$", description: "decimal" };
    case "string":
      return {
        type: "string",
        ...(schema.maxLength === undefined ? {} : { maxLength: schema.maxLength }),
      };
    case "enum":
      return { type: "string", enum: schema.values };
    case "array":
      return { type: "array", items: responseSchema(schema.items), maxItems: schema.maxItems };
    case "object":
      return {
        type: "object",
        properties: Object.fromEntries(
          Object.entries(schema.fields).map(([n, f]) => [n, responseSchema(f)]),
        ),
        required: Object.keys(schema.fields),
      };
    case "nullable":
      return { anyOf: [responseSchema(schema.of), { type: "null" }] };
    case "oneOf":
      return { oneOf: schema.variants.map(responseSchema) };
  }
}

const ERROR_BODY: JsonSchema = {
  type: "object",
  properties: { error: { type: "string" }, message: { type: "string" }, details: {} },
  required: ["error", "message"],
};

function objectSchema(fields: Record<string, Field>): JsonSchema {
  const required = Object.entries(fields)
    .filter(([, f]) => !f.optional && f.default === undefined)
    .map(([name]) => name);
  return {
    type: "object",
    properties: Object.fromEntries(
      Object.entries(fields).map(([n, f]) => [n, fieldSchema(f.type)]),
    ),
    ...(required.length > 0 ? { required } : {}),
    additionalProperties: false,
  };
}

function operation(route: Route, report: CheckReport | undefined): JsonSchema {
  const parameters = [
    ...Object.entries(route.input.params).map(([name, f]) => ({
      name,
      in: "path",
      required: true,
      schema: fieldSchema(f.type),
    })),
    ...Object.entries(route.input.query).map(([name, f]) => ({
      name,
      in: "query",
      required: !f.optional && f.default === undefined,
      schema: fieldSchema(f.type),
    })),
  ];
  const first = route.plan.body[0];
  const status = first?.kind === "setStatus" ? first.status : 200;
  const output = report?.routes[route.id]?.output;
  const errorBody = { content: { "application/json": { schema: ERROR_BODY } } };
  const responses: Record<string, JsonSchema> = {
    [String(status)]:
      status === 204
        ? { description: "No content" }
        : {
            description: "OK",
            ...(output
              ? { content: { "application/json": { schema: responseSchema(output) } } }
              : {}),
          },
  };
  if (route.auth.kind !== "public" || route.webhook) {
    responses["401"] = {
      description: route.webhook ? "Invalid signature" : "Not signed in",
      ...errorBody,
    };
  }
  if (route.auth.kind === "member") responses["403"] = { description: "Not allowed", ...errorBody };
  responses["400"] = { description: "Invalid input", ...errorBody };
  if (route.rateLimit) responses["429"] = { description: "Rate limited", ...errorBody };
  for (const [name, error] of Object.entries(route.errors)) {
    const details = error.details ? { details: objectSchema(error.details) } : {};
    responses[String(error.status)] = {
      description: `${name}: ${error.message}`,
      content: {
        "application/json": {
          schema: {
            ...ERROR_BODY,
            properties: {
              ...(ERROR_BODY["properties"] as object),
              error: { const: name },
              ...details,
            },
          },
        },
      },
    };
  }
  return {
    operationId: route.id,
    ...(route.description ? { summary: route.description } : {}),
    ...(parameters.length > 0 ? { parameters } : {}),
    ...(Object.keys(route.input.body).length > 0
      ? {
          requestBody: {
            required: true,
            content: { "application/json": { schema: objectSchema(route.input.body) } },
          },
        }
      : {}),
    ...(route.deprecated ? { deprecated: true, "x-aleph-deprecated": route.deprecated } : {}),
    ...(route.version ? { "x-aleph-version": route.version } : {}),
    ...(route.webhook ? { "x-aleph-webhook": route.webhook } : {}),
    ...(route.examples?.length
      ? {
          "x-aleph-examples": route.examples.map((e) => ({
            name: e.name,
            request: e.request,
            expect: e.expect,
          })),
        }
      : {}),
    ...(route.idempotency
      ? {
          "x-aleph-idempotency": {
            ttlMs: route.idempotency.ttlMs,
            required: route.idempotency.required ?? false,
          },
        }
      : {}),
    responses,
  };
}

/** An OpenAPI 3.1 document for the manifest's routes, with response schemas from `report`. */
export function openapi(manifest: Manifest, report?: CheckReport): JsonSchema {
  const paths: Record<string, Record<string, JsonSchema>> = {};
  for (const route of manifest.routes) {
    const path = route.path.replace(/:(\w+)/g, "{$1}");
    paths[path] = { ...paths[path], [route.method.toLowerCase()]: operation(route, report) };
  }
  return {
    openapi: "3.1.0",
    info: { title: manifest.app.name, version: manifest.formatVersion },
    paths,
  };
}

/** MCP tool descriptions for the routes marked with `.mcp(name)`. */
export function mcpTools(manifest: Manifest): JsonSchema[] {
  return manifest.routes.flatMap((route) => {
    if (!route.mcp) return [];
    const input = { ...route.input.params, ...route.input.query, ...route.input.body };
    return [
      {
        name: route.mcp.name,
        description: route.description ?? `${route.method} ${route.path}`,
        inputSchema: objectSchema(input),
        "x-aleph-route": route.id,
      },
    ];
  });
}
