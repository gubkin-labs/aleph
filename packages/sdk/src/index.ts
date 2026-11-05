/**
 * `@aleph/sdk`: declare a backend in TypeScript and build it into an Aleph manifest.
 *
 * Handlers run once, at build time, with symbolic values. Every operation they call is recorded
 * into a plan, and the plan, not your code, is what the platform runs.
 *
 * @example
 * ```ts
 * import { app, err, t } from "@aleph/sdk";
 *
 * const a = app("todo");
 * const Todo = a.entity("Todo", {
 *   key: ["ownerId", "todoId"],
 *   fields: { ownerId: t.string().max(64), todoId: t.id().generated(), title: t.string().min(1).max(200) },
 * });
 * a.route("POST /todos")
 *   .body({ title: t.string().min(1).max(200) })
 *   .handle(($, { body }) => Todo.insert({ ownerId: $.caller, title: body.title }));
 * ```
 */
export { App, app } from "./app.js";
export {
  Counter,
  type CounterDef,
  Entity,
  type EntityDef,
  ErrorRef,
  type InsertValues,
  type KeyValues,
  List,
  type QueryOptions,
  type Row,
  type RowMethods,
  type ValuesLike,
} from "./data.js";
export { $, type MatchCases } from "./dsl.js";
export { err } from "./errors.js";
export { Expr, type ExprLike } from "./expr.js";
export {
  type AnyField,
  FieldBuilder,
  type Fields,
  type FieldValue,
  IdField,
  IntField,
  type RequiredFields,
  StringField,
  StringNeedsMax,
  TimestampField,
  t,
  type Values,
} from "./fields.js";
export { stableStringify } from "./json.js";
export { AlephBuildError } from "./recorder.js";
export type {
  Handler,
  HandlerContext,
  InputValue,
  InputValues,
  PathParams,
  RouteBuilder,
} from "./route.js";
