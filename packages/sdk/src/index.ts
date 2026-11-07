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
export {
  App,
  type AppOptions,
  app,
  type BuildOptions,
  type UserValue,
  verify,
  type WebhookOptions,
} from "./app.js";
export {
  type Accessor,
  type ComputedGroup,
  Counter,
  type CounterAggDef,
  type CounterDef,
  type CounterValue,
  Entity,
  type EntityDef,
  ErrorRef,
  type IndexDef,
  type IndexField,
  type InsertValues,
  type KeyValues,
  type Lifted,
  List,
  type LoopOptions,
  type LoopState,
  type ObjectAccessor,
  Page,
  type ParentDef,
  type QueryOptions,
  type RangeBounds,
  type Row,
  type RowMethods,
  type UpdateOptions,
  type ValueLike,
  type ValuesLike,
} from "./data.js";
export { $, type CookieOptions, type MatchCases } from "./dsl.js";
export {
  type ChangePayload,
  Channel,
  type CronPayload,
  Event,
  type HandlerCtx,
  type HandlerOptions,
  type IntegrationDef,
  IntegrationHandle,
  type OperationDef,
  Queue,
  type QueueOptions,
  type ScanOptions,
} from "./effects.js";
export { err } from "./errors.js";
export { Expr, type ExprLike, type Numeric, type TimeZoneLike } from "./expr.js";
export {
  type AnyField,
  type Decimal,
  DecimalField,
  FieldBuilder,
  type Fields,
  type FieldValue,
  IdField,
  IntField,
  type ObjectValue,
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
  ExampleDef,
  Handler,
  HandlerContext,
  InputValue,
  InputValues,
  PathParams,
  RouteBuilder,
} from "./route.js";
export type { Duration } from "./units.js";
export {
  type StepContext,
  type StepOptions,
  Workflow,
  type WorkflowDef,
  type WorkflowInstance,
} from "./workflow.js";
