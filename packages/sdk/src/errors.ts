/** Declared business errors: `err.notFound()`, `err.forbidden("...")`, ... */
import type { ErrorDecl } from "@aleph/manifest";
import { type Fields, fieldSpecs } from "./fields.js";

/**
 * Creates an error declaration, optionally with a custom message and the `details` it carries
 * (`err.unprocessable("Insufficient funds", { balance: t.int(), needed: t.int() })`). Every
 * `$.fail` and `$.ensure` of the error then passes those details.
 */
export type ErrorFactory = (message?: string, details?: Fields) => ErrorDecl;

const decl =
  (status: number, fallback: string): ErrorFactory =>
  (message = fallback, details) => ({
    status,
    message,
    ...(details && Object.keys(details).length > 0 ? { details: fieldSpecs(details) } : {}),
  });

/** Error declarations for `route.errors({...})`. */
export const err: {
  badRequest: ErrorFactory;
  unauthorized: ErrorFactory;
  forbidden: ErrorFactory;
  notFound: ErrorFactory;
  conflict: ErrorFactory;
  gone: ErrorFactory;
  unprocessable: ErrorFactory;
  tooManyRequests: ErrorFactory;
  status: (status: number, message: string, details?: Fields) => ErrorDecl;
} = {
  badRequest: decl(400, "Bad request"),
  unauthorized: decl(401, "Unauthorized"),
  forbidden: decl(403, "Forbidden"),
  notFound: decl(404, "Not found"),
  conflict: decl(409, "Conflict"),
  gone: decl(410, "Gone"),
  unprocessable: decl(422, "Unprocessable"),
  tooManyRequests: decl(429, "Too many requests"),
  /** A custom status (400–599). */
  status: (status, message, details) => decl(status, message)(message, details),
};
