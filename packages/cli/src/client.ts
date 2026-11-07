/**
 * `aleph client`: a typed fetch client generated from a manifest and its check report. Every
 * route becomes a function with typed inputs, a typed result (the declared `returns`, or what the
 * checker inferred), and typed errors, so changing a route breaks the code that calls it.
 */
import type {
  CheckReport,
  Field,
  FieldType,
  Manifest,
  Route,
  TypeSchema,
} from "@aleph-lang/manifest";

const IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

function key(name: string): string {
  return IDENTIFIER.test(name) ? name : JSON.stringify(name);
}

/** The TypeScript type of a JSON value with this schema. */
export function schemaType(schema: TypeSchema): string {
  switch (schema.kind) {
    case "any":
      return "unknown";
    case "null":
      return "null";
    case "bool":
      return "boolean";
    case "int":
      return "number";
    case "decimal":
      return "Decimal";
    case "string":
      return "string";
    case "enum":
      return schema.values.map((v) => JSON.stringify(v)).join(" | ");
    case "array":
      return `${wrap(schemaType(schema.items))}[]`;
    case "object": {
      const fields = Object.entries(schema.fields).map(
        ([name, field]) => `${key(name)}: ${schemaType(field)}`,
      );
      return fields.length === 0 ? "Record<string, never>" : `{ ${fields.join("; ")} }`;
    }
    case "nullable":
      return `${wrap(schemaType(schema.of))} | null`;
    case "oneOf":
      return schema.variants.map((v) => wrap(schemaType(v))).join(" | ");
  }
}

/** The TypeScript type a client sends for a field of this type. */
export function inputType(type: FieldType): string {
  switch (type.kind) {
    case "string":
    case "id":
    case "timeZone":
      return "string";
    case "int":
    case "timestamp":
      return "number";
    case "decimal":
      return "Decimal | number";
    case "bool":
      return "boolean";
    case "enum":
      return type.values.map((v) => JSON.stringify(v)).join(" | ");
    case "array":
      return `${wrap(inputType(type.items))}[]`;
    case "object":
      return objectInput(type.fields);
  }
}

function wrap(type: string): string {
  return /[|&\s]/.test(type) && !type.startsWith("{") ? `(${type})` : type;
}

function optional(field: Field): boolean {
  return field.optional === true || field.default !== undefined;
}

function objectInput(fields: Record<string, Field>): string {
  const parts = Object.entries(fields).map(([name, field]) => {
    const type = inputType(field.type);
    return optional(field)
      ? `${key(name)}?: ${field.optional ? `${wrap(type)} | null` : type}`
      : `${key(name)}: ${type}`;
  });
  return parts.length === 0 ? "Record<string, never>" : `{ ${parts.join("; ")} }`;
}

/** `post_orgs_orgId_projects` → `postOrgsOrgIdProjects`. */
function camel(id: string): string {
  const out = id.replace(/[_-]+([A-Za-z0-9])/g, (_, c: string) => c.toUpperCase());
  return IDENTIFIER.test(out) ? out : `route_${out.replace(/[^A-Za-z0-9_$]/g, "_")}`;
}

function routeInput(route: Route): string {
  const all: Record<string, Field> = {
    ...route.input.params,
    ...route.input.query,
    ...route.input.body,
  };
  return objectInput(all);
}

function errorType(route: Route): string {
  const errors = Object.entries(route.errors).map(([name, decl]) => {
    const details = decl.details ? `; details: ${objectOutput(decl.details)}` : "";
    return `{ error: ${JSON.stringify(name)}; message: string${details} }`;
  });
  return [...errors, "AlephErrorBody"].join(" | ");
}

function objectOutput(fields: Record<string, Field>): string {
  const parts = Object.entries(fields).map(([name, field]) => {
    const type = inputType(field.type).replace("Decimal | number", "Decimal");
    return `${key(name)}: ${field.optional ? `${wrap(type)} | null` : type}`;
  });
  return `{ ${parts.join("; ")} }`;
}

function successStatus(route: Route): number {
  const first = route.plan.body[0];
  return first?.kind === "setStatus" ? first.status : 200;
}

/** Generates the client module's source. */
export function generateClient(manifest: Manifest, report: CheckReport): string {
  const names = new Set<string>();
  const routes = manifest.routes.map((route) => {
    let name = camel(route.id);
    while (names.has(name)) name = `${name}_`;
    names.add(name);
    const output = report.routes[route.id]?.output ?? { kind: "any" };
    const status = successStatus(route);
    return { route, name, output: status === 204 ? "null" : schemaType(output) };
  });

  const types = routes
    .map(
      ({ route, name, output }) =>
        `  /** ${route.method} ${route.path}${route.description ? ` — ${route.description}` : ""}${route.deprecated ? ` @deprecated ${route.deprecated}` : ""} */\n` +
        `  ${key(name)}: {\n` +
        `    input: ${routeInput(route)};\n` +
        `    output: ${output};\n` +
        `    error: ${errorType(route)};\n` +
        "  };",
    )
    .join("\n");

  const methods = routes
    .map(({ route, name }) => {
      const spec = {
        method: route.method,
        path: route.path,
        params: Object.keys(route.input.params),
        query: Object.keys(route.input.query),
        body: Object.keys(route.input.body),
      };
      const doc = route.deprecated ? `    /** @deprecated ${route.deprecated} */\n` : "";
      return `${doc}    ${key(name)}: (input: Routes[${JSON.stringify(name)}]["input"], options?: CallOptions) =>\n      call<Routes[${JSON.stringify(name)}]>(${JSON.stringify(spec)}, input, options),`;
    })
    .join("\n");

  return `// Generated by \`aleph client\` from the ${JSON.stringify(manifest.app.name)} app. Don't edit: regenerate.
/* eslint-disable */

/** An exact decimal, as JSON carries it: a string like "12.50". */
export type Decimal = \`\${number}\`;

/** An error response body. */
export interface AlephErrorBody {
  error: string;
  message: string;
  details?: unknown;
}

/** A failed call: the status and the (typed) error body. */
export class AlephError<E = AlephErrorBody> extends Error {
  readonly status: number;
  readonly body: E;

  constructor(status: number, body: E) {
    const message = (body as { message?: unknown }).message;
    super(typeof message === "string" ? message : \`request failed with \${status}\`);
    this.name = "AlephError";
    this.status = status;
    this.body = body;
  }
}

/** How the client reaches the app. */
export interface ClientOptions {
  /** Where the app is served, like "http://127.0.0.1:3000". */
  baseUrl: string;
  /** A fetch implementation (defaults to the global one). */
  fetch?: typeof fetch;
  /** Headers sent with every call (auth tokens, dev caller headers). */
  headers?: () => Record<string, string> | Promise<Record<string, string>>;
}

/** Per-call options. */
export interface CallOptions {
  /** Sent as Idempotency-Key, for routes that replay repeated requests. */
  idempotencyKey?: string;
  headers?: Record<string, string>;
  signal?: AbortSignal;
}

/** Every route: its input, its successful result, and its errors. */
export interface Routes {
${types}
}

interface RouteSpec {
  method: string;
  path: string;
  params: string[];
  query: string[];
  body: string[];
}

/** Creates a client for the app. */
export function createClient(options: ClientOptions) {
  const doFetch = options.fetch ?? fetch;
  async function call<R extends { output: unknown; error: unknown }>(
    spec: RouteSpec,
    input: object,
    callOptions: CallOptions = {},
  ): Promise<R["output"]> {
    const values = input as Record<string, unknown>;
    const path = spec.path.replace(/:([A-Za-z0-9_]+)/g, (_, name: string) =>
      encodeURIComponent(String(values[name])),
    );
    const query = new URLSearchParams();
    for (const name of spec.query) {
      const value = values[name];
      if (value !== undefined && value !== null) query.set(name, String(value));
    }
    const body: Record<string, unknown> = {};
    for (const name of spec.body) if (values[name] !== undefined) body[name] = values[name];
    const headers: Record<string, string> = {
      ...(options.headers ? await options.headers() : {}),
      ...callOptions.headers,
    };
    if (callOptions.idempotencyKey) headers["idempotency-key"] = callOptions.idempotencyKey;
    const hasBody = spec.body.length > 0;
    if (hasBody) headers["content-type"] = "application/json";
    const search = query.size > 0 ? \`?\${query}\` : "";
    const response = await doFetch(\`\${options.baseUrl.replace(/\\/$/, "")}\${path}\${search}\`, {
      method: spec.method,
      headers,
      ...(hasBody ? { body: JSON.stringify(body) } : {}),
      ...(callOptions.signal ? { signal: callOptions.signal } : {}),
    });
    const text = await response.text();
    const json: unknown = text === "" ? null : JSON.parse(text);
    if (response.status >= 400) throw new AlephError<R["error"]>(response.status, json as R["error"]);
    return json as R["output"];
  }

  return {
${methods}
  };
}

/** The client's type. */
export type Client = ReturnType<typeof createClient>;
`;
}
