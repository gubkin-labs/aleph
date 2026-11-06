/**
 * Routes: `a.route("POST /orgs/:orgId/projects").body({...}).errors({...}).handle(($, ctx) => ...)`.
 */
import type {
  Auth,
  Caller,
  ErrorDecl,
  Field,
  FieldType,
  Idempotency,
  InputSource,
  Method,
  RateLimit,
  Route,
  RouteExample,
  Webhook,
} from "@aleph/manifest";
import { type Accessor, ErrorRef, materialize, shapeOfType } from "./data.js";
import { $ } from "./dsl.js";
import { type FieldBuilder, type Fields, type FieldValue, fieldSpecs, t } from "./fields.js";
import { toNode } from "./node.js";
import { AlephBuildError, callSite, Recorder, recording } from "./recorder.js";
import { type Duration, durationMs } from "./units.js";

const METHODS = new Set<Method>(["GET", "POST", "PUT", "PATCH", "DELETE"]);

/** The `:param` names of a path template. */
export type PathParams<P extends string> = P extends `${string}:${infer Param}/${infer Rest}`
  ? Param | PathParams<`/${Rest}`>
  : P extends `${string}:${infer Param}`
    ? Param
    : never;

/** Path params default to strings of at most 128 characters. */
export type DefaultParams<P extends string> = { [K in PathParams<P>]: FieldBuilder<string> };

/** What a handler sees for one input field: lists, objects of accessors, or expressions. */
export type InputValue<F> =
  F extends FieldBuilder<unknown, boolean, boolean> ? Accessor<FieldValue<F>> : never;

/** What a handler sees for a record of input fields. */
export type InputValues<F extends Fields> = { readonly [K in keyof F]: InputValue<F[K]> };

/** The handler's context. */
export interface HandlerContext<
  P extends Fields,
  Q extends Fields,
  B extends Fields,
  E extends string,
> {
  params: InputValues<P>;
  query: InputValues<Q>;
  body: InputValues<B>;
  errors: { readonly [K in E]: ErrorRef };
}

/** A route handler. Runs once, at build time, to record the plan. */
export type Handler<P extends Fields, Q extends Fields, B extends Fields, E extends string> = (
  $dsl: typeof $,
  ctx: HandlerContext<P, Q, B, E>,
) => unknown;

export { type Duration, durationMs } from "./units.js";

/** @internal What a route builder hands back to its app. */
export interface RouteSink {
  /** The index the next route will have. */
  next(): number;
  add(route: Route, sourceMap: Record<string, string>): void;
}

/** @internal Input expressions for declared fields. */
export function inputs<F extends Fields>(source: InputSource, fields: F): InputValues<F> {
  const out: Record<string, unknown> = {};
  for (const [name, field] of Object.entries(fields)) {
    out[name] = materialize(shapeOfType(field.spec.type), { kind: "input", source, name });
  }
  return out as InputValues<F>;
}

function defaultId(method: Method, path: string): string {
  const segments = path
    .split("/")
    .filter(Boolean)
    .map((s) => s.replace(/^:/, "").replace(/[^A-Za-z0-9_]/g, "_"));
  return [method.toLowerCase(), ...segments].join("_");
}

/** Builds one route. Nothing is registered until {@link RouteBuilder.handle} is called. */
export class RouteBuilder<
  P extends Fields,
  Q extends Fields,
  B extends Fields,
  E extends string,
  R extends string = string,
  S extends string = string,
> {
  readonly #sink: RouteSink;
  readonly #method: Method;
  readonly #path: string;
  #id: string;
  #auth: Auth = { kind: "signedIn" };
  #params: Fields;
  #query: Fields = {};
  #body: Fields = {};
  #errors: Record<string, ErrorDecl> = {};
  #idempotency: Idempotency | undefined;
  #description: string | undefined;
  #mcp: string | undefined;
  #rateLimit: RateLimit | undefined;
  #webhook: Webhook | undefined;
  #returns: FieldType | undefined;
  #deprecated: string | undefined;
  #version: string | undefined;
  #status: number | undefined;
  #examples: RouteExample[] = [];

  /** @internal */
  constructor(sink: RouteSink, spec: string) {
    const [method, path, ...rest] = spec.trim().split(/\s+/);
    if (!method || !path || rest.length > 0 || !METHODS.has(method as Method)) {
      throw new AlephBuildError(`route ${JSON.stringify(spec)} must look like "POST /path/:param"`);
    }
    this.#sink = sink;
    this.#method = method as Method;
    this.#path = path;
    this.#id = defaultId(this.#method, path);
    this.#params = Object.fromEntries(
      path
        .split("/")
        .filter((s) => s.startsWith(":"))
        .map((s) => [s.slice(1), t.string().max(128)]),
    );
  }

  /** A stable id for the route. Defaults to method and path, like `post_orgs_orgId_projects`. */
  id(id: string): this {
    this.#id = id;
    return this;
  }

  /** Who may call the route. Defaults to `"signedIn"`. */
  auth(kind: "public" | "signedIn"): this {
    this.#auth = { kind };
    return this;
  }
  /**
   * Only members of the org in path param `org` may call the route, optionally only with some
   * roles. API keys for that org are accepted only if `apiKeyScopes` is set, and only with all of
   * those scopes.
   */
  member(
    org: keyof P & string,
    options: { roles?: readonly R[]; apiKeyScopes?: readonly S[] } = {},
  ): this {
    this.#auth = {
      kind: "member",
      org,
      roles: [...(options.roles ?? [])],
      apiKeyScopes: [...(options.apiKeyScopes ?? [])],
    };
    return this;
  }

  /**
   * Replays the stored response when a request repeats its `Idempotency-Key` within `ttl`
   * (`"30s"`, `"15m"`, `"24h"`, `"7d"`, or milliseconds).
   */
  /** What the route does, for generated API docs and MCP tools. */
  describe(description: string): this {
    this.#description = description;
    return this;
  }

  /** Also exposes the route as an MCP tool, with the same inputs, auth, and limits. */
  mcp(tool: string): this {
    this.#mcp = tool;
    return this;
  }

  /** At most `max` calls per `per` (user, IP, API key, or everyone) in each `window`. */
  rateLimit(options: { per: RateLimit["per"]; max: number; window: Duration }): this {
    this.#rateLimit = { per: options.per, max: options.max, windowMs: durationMs(options.window) };
    return this;
  }

  /** @internal Makes the route a webhook (see `App.webhook`). */
  webhookConfig(webhook: Webhook): this {
    this.#webhook = webhook;
    return this;
  }

  /**
   * Declares the response type. The checker verifies every response fits it, fields the type
   * doesn't list are dropped from the response, and generated clients and docs use it.
   */
  returns(type: FieldBuilder<unknown, boolean, boolean>): this {
    this.#returns = type.spec.type;
    return this;
  }

  /** The success status, like 201 for creates or 204 for no content (the body is dropped). */
  status(status: 200 | 201 | 202 | 204): this {
    this.#status = status;
    return this;
  }

  /** Marks the route deprecated. Responses carry `Deprecation: true`, and docs mark it. */
  deprecated(note: string): this {
    this.#deprecated = note;
    return this;
  }

  /** An API version label for docs, like `"2024-06-01"`. */
  version(version: string): this {
    this.#version = version;
    return this;
  }

  /**
   * A worked example: given some rows, this request gets this response. `aleph test` runs every
   * example; docs show them. Bodies match as subsets (listed keys only).
   */
  example(example: ExampleDef<P, Q, B>): this {
    const { as, ...rest } = example;
    this.#examples.push(
      structuredClone({
        ...rest,
        request: rest.request ?? {},
        expect: rest.expect ?? {},
        ...(as === undefined ? {} : { caller: as }),
      }) as RouteExample,
    );
    return this;
  }

  idempotent(options: { ttl: Duration; required?: boolean }): this {
    this.#idempotency = {
      ttlMs: durationMs(options.ttl),
      ...(options.required ? { required: true } : {}),
    };
    return this;
  }

  /** Overrides the types of path params (they default to strings of at most 128 characters). */
  params<NP extends { [K in keyof P]: FieldBuilder<unknown, false, boolean> }>(
    fields: NP,
  ): RouteBuilder<NP, Q, B, E, R, S> {
    this.#params = { ...this.#params, ...fields };
    return this as unknown as RouteBuilder<NP, Q, B, E, R, S>;
  }

  /** Declares query string parameters. */
  query<NQ extends Fields>(fields: NQ): RouteBuilder<P, NQ, B, E, R, S> {
    this.#query = fields;
    return this as unknown as RouteBuilder<P, NQ, B, E, R, S>;
  }

  /** Declares JSON body fields. */
  body<NB extends Fields>(fields: NB): RouteBuilder<P, Q, NB, E, R, S> {
    this.#body = fields;
    return this as unknown as RouteBuilder<P, Q, NB, E, R, S>;
  }

  /** Declares the business errors the handler may fail with. */
  errors<NE extends string>(errors: Record<NE, ErrorDecl>): RouteBuilder<P, Q, B, NE, R, S> {
    this.#errors = errors;
    return this as unknown as RouteBuilder<P, Q, B, NE, R, S>;
  }

  /** Records the handler's plan and registers the route. */
  handle(handler: Handler<P, Q, B, E>): void {
    const declared = Object.fromEntries(
      Object.keys(this.#errors).map((name) => [name, new ErrorRef(name)]),
    );
    // Reading an undeclared error is a build error, not a silent `undefined`.
    const errors = new Proxy(declared, {
      get: (target, prop) => {
        if (typeof prop === "string" && !(prop in target) && prop !== "then" && prop !== "toJSON") {
          throw new AlephBuildError(
            `route ${this.#method} ${this.#path} has no error ${JSON.stringify(prop)}; ` +
              `declare it with .errors({ ${prop}: err.badRequest("...") })`,
          );
        }
        return Reflect.get(target, prop);
      },
    });
    const ctx = {
      params: inputs("params", this.#params as P),
      query: inputs("query", this.#query as Q),
      body: inputs("body", this.#body as B),
      errors: errors as HandlerContext<P, Q, B, E>["errors"],
    };
    const index = this.#sink.next();
    const rec = new Recorder(`routes[${index}].plan.body`);
    const site = callSite();
    if (site) rec.sourceMap[`routes[${index}]`] = site;
    const status = this.#status;
    const output = recording(rec, () => {
      if (status !== undefined && status !== 200) rec.push({ kind: "setStatus", status });
      return toNode(handler($, ctx));
    });
    const route: Route = {
      id: this.#id,
      method: this.#method,
      path: this.#path,
      auth: this.#auth,
      input: { params: specs(this.#params), query: specs(this.#query), body: specs(this.#body) },
      errors: { ...this.#errors },
      ...(this.#idempotency ? { idempotency: this.#idempotency } : {}),
      ...(this.#description ? { description: this.#description } : {}),
      ...(this.#mcp ? { mcp: { name: this.#mcp } } : {}),
      ...(this.#rateLimit ? { rateLimit: this.#rateLimit } : {}),
      ...(this.#webhook ? { webhook: this.#webhook } : {}),
      ...(this.#returns ? { returns: this.#returns } : {}),
      ...(this.#deprecated ? { deprecated: this.#deprecated } : {}),
      ...(this.#version ? { version: this.#version } : {}),
      ...(this.#examples.length > 0 ? { examples: this.#examples } : {}),
      plan: { body: rec.body, output },
    };
    this.#sink.add(route, rec.sourceMap);
  }
}

/** Plain JSON values for a record of fields, as a client sends them. */
type JsonValues<F extends Fields> = { [K in keyof F]?: FieldValue<F[K]> };

/** A worked example of a route (see {@link RouteBuilder.example}). */
export interface ExampleDef<P extends Fields, Q extends Fields, B extends Fields> {
  /** What the example shows. */
  name: string;
  /** Rows that exist before the request, by entity name. */
  given?: Record<string, readonly Record<string, unknown>[]>;
  /** User profiles in the directory, by user id. */
  users?: Record<string, Record<string, unknown>>;
  /** Test secret values, such as a webhook signing key (never real ones: examples ship in the manifest). */
  secrets?: Record<string, string>;
  /** Who calls (anonymous if omitted). */
  as?: Caller;
  /** The request. */
  request?: {
    params?: { [K in keyof P]?: string };
    query?: { [K in keyof Q]?: string };
    body?: JsonValues<B> | Record<string, unknown>;
    headers?: Record<string, string>;
  };
  /** Integration responses for calls the request makes. */
  mocks?: readonly { integration: string; operation: string; response?: unknown; error?: string }[];
  /** Run the async work the request caused before checking `runs` and `deliveries`. */
  process?: boolean;
  /** What should happen. */
  expect?: {
    status?: number;
    /** A subset of the response body (or an error body: `{ error: "<name>" }`). */
    body?: unknown;
    /** The handlers that ran (with `process`). */
    runs?: readonly string[];
    /** The deliveries made, each a subset (with `process`). */
    deliveries?: readonly unknown[];
  };
}

function specs(fields: Fields): Record<string, Field> {
  return fieldSpecs(fields);
}
