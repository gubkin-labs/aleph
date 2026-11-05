/**
 * Routes: `a.route("POST /orgs/:orgId/projects").body({...}).errors({...}).handle(($, ctx) => ...)`.
 */
import type { Auth, ErrorDecl, Field, InputSource, Method, Route } from "@aleph/manifest";
import { ErrorRef, List } from "./data.js";
import { $ } from "./dsl.js";
import { Expr } from "./expr.js";
import { type FieldBuilder, type Fields, type FieldValue, fieldSpecs, t } from "./fields.js";
import { toNode } from "./node.js";
import { AlephBuildError, Recorder, recording } from "./recorder.js";

const METHODS = new Set<Method>(["GET", "POST", "PUT", "PATCH", "DELETE"]);

/** The `:param` names of a path template. */
export type PathParams<P extends string> = P extends `${string}:${infer Param}/${infer Rest}`
  ? Param | PathParams<`/${Rest}`>
  : P extends `${string}:${infer Param}`
    ? Param
    : never;

/** Path params default to strings of at most 128 characters. */
export type DefaultParams<P extends string> = { [K in PathParams<P>]: FieldBuilder<string> };

/** What a handler sees for one input field. */
export type InputValue<F> =
  F extends FieldBuilder<infer V, boolean, boolean>
    ? V extends readonly (infer U)[]
      ? List<Expr<U>>
      : Expr<FieldValue<F>>
    : never;

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

/** @internal What a route builder hands back to its app. */
export type RouteSink = (route: Route) => void;

function inputs<F extends Fields>(source: InputSource, fields: F): InputValues<F> {
  const out: Record<string, unknown> = {};
  for (const [name, field] of Object.entries(fields)) {
    const node = { kind: "input" as const, source, name };
    out[name] =
      field.spec.type.kind === "array" ? new List(node, { kind: "scalar" }) : new Expr(node);
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
export class RouteBuilder<P extends Fields, Q extends Fields, B extends Fields, E extends string> {
  readonly #sink: RouteSink;
  readonly #method: Method;
  readonly #path: string;
  #id: string;
  #auth: Auth = { kind: "signedIn" };
  #params: Fields;
  #query: Fields = {};
  #body: Fields = {};
  #errors: Record<string, ErrorDecl> = {};

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
  auth(kind: Auth["kind"]): this {
    this.#auth = { kind };
    return this;
  }

  /** Overrides the types of path params (they default to strings of at most 128 characters). */
  params<NP extends { [K in keyof P]: FieldBuilder<unknown, false, boolean> }>(
    fields: NP,
  ): RouteBuilder<NP, Q, B, E> {
    this.#params = { ...this.#params, ...fields };
    return this as unknown as RouteBuilder<NP, Q, B, E>;
  }

  /** Declares query string parameters. */
  query<NQ extends Fields>(fields: NQ): RouteBuilder<P, NQ, B, E> {
    this.#query = fields;
    return this as unknown as RouteBuilder<P, NQ, B, E>;
  }

  /** Declares JSON body fields. */
  body<NB extends Fields>(fields: NB): RouteBuilder<P, Q, NB, E> {
    this.#body = fields;
    return this as unknown as RouteBuilder<P, Q, NB, E>;
  }

  /** Declares the business errors the handler may fail with. */
  errors<NE extends string>(errors: Record<NE, ErrorDecl>): RouteBuilder<P, Q, B, NE> {
    this.#errors = errors;
    return this as unknown as RouteBuilder<P, Q, B, NE>;
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
    const rec = new Recorder();
    const output = recording(rec, () => toNode(handler($, ctx)));
    this.#sink({
      id: this.#id,
      method: this.#method,
      path: this.#path,
      auth: this.#auth,
      input: { params: specs(this.#params), query: specs(this.#query), body: specs(this.#body) },
      errors: { ...this.#errors },
      plan: { body: rec.body, output },
    });
  }
}

function specs(fields: Fields): Record<string, Field> {
  return fieldSpecs(fields);
}
