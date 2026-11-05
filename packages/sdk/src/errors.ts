/** Declared business errors: `err.notFound()`, `err.forbidden("...")`, ... */
import type { ErrorDecl } from "@aleph/manifest";

/** Creates an error declaration, optionally with a custom message. */
export type ErrorFactory = (message?: string) => ErrorDecl;

const decl =
  (status: number, fallback: string): ErrorFactory =>
  (message = fallback) => ({ status, message });

/** Error declarations for `route.errors({...})`. */
export const err: {
  badRequest: ErrorFactory;
  forbidden: ErrorFactory;
  notFound: ErrorFactory;
  conflict: ErrorFactory;
  unprocessable: ErrorFactory;
  tooManyRequests: ErrorFactory;
  status: (status: number, message: string) => ErrorDecl;
} = {
  badRequest: decl(400, "Bad request"),
  forbidden: decl(403, "Forbidden"),
  notFound: decl(404, "Not found"),
  conflict: decl(409, "Conflict"),
  unprocessable: decl(422, "Unprocessable"),
  tooManyRequests: decl(429, "Too many requests"),
  /** A custom status (400–599). */
  status: (status, message) => ({ status, message }),
};
