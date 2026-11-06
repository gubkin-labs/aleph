/** OpenAPI and MCP descriptions generated from a manifest. */
import type { Field, FieldType, Manifest, Route } from "@aleph/manifest";

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
  }
}

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

function operation(route: Route): JsonSchema {
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
  const responses: Record<string, JsonSchema> = { "200": { description: "OK" } };
  if (route.auth.kind !== "public") {
    responses["401"] = { description: "Not signed in" };
  }
  if (route.auth.kind === "member") responses["403"] = { description: "Not allowed" };
  responses["400"] = { description: "Invalid input" };
  if (route.rateLimit) responses["429"] = { description: "Rate limited" };
  for (const [name, error] of Object.entries(route.errors)) {
    responses[String(error.status)] = { description: `${name}: ${error.message}` };
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

/** An OpenAPI 3.1 document for the manifest's routes. */
export function openapi(manifest: Manifest): JsonSchema {
  const paths: Record<string, Record<string, JsonSchema>> = {};
  for (const route of manifest.routes) {
    const path = route.path.replace(/:(\w+)/g, "{$1}");
    paths[path] = { ...paths[path], [route.method.toLowerCase()]: operation(route) };
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
