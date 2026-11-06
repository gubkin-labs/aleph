/**
 * Types for the Aleph manifest format.
 *
 * Everything in `./generated` is produced by `cargo xtask codegen` from the Rust types in
 * `crates/aleph-core`, which are the single source of truth for the format.
 */
export type * from "./generated.js";

/** The manifest format version these types describe. */
export const FORMAT_VERSION = "0.2" as const;
