/** Durations: `"30s"`, `"15m"`, `"24h"`, `"7d"`, or milliseconds. */
import { AlephBuildError } from "./recorder.js";

/** A duration: milliseconds, or a number with a unit (`"30s"`, `"15m"`, `"24h"`, `"7d"`). */
export type Duration = number | `${number}${"s" | "m" | "h" | "d"}`;

const UNIT_MS = { s: 1_000, m: 60_000, h: 3_600_000, d: 86_400_000 } as const;

/** Converts a {@link Duration} to milliseconds. */
export function durationMs(duration: Duration): number {
  if (typeof duration === "number") return duration;
  const match = /^(\d+)([smhd])$/.exec(duration);
  const unit = match?.[2] as keyof typeof UNIT_MS | undefined;
  if (!match?.[1] || !unit)
    throw new AlephBuildError(`invalid duration ${JSON.stringify(duration)}`);
  return Number(match[1]) * UNIT_MS[unit];
}
