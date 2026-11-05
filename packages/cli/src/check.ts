/** `aleph check`: diagnostics, or a per-route cost table. */
import { defaultLimits, validate } from "@aleph/core";
import type { CheckReport, Cost, Diagnostic } from "@aleph/manifest";

/** Formats diagnostics for a terminal. */
export function formatDiagnostics(diagnostics: Diagnostic[]): string {
  const lines = diagnostics.map((d) => `✖ ${d.code} at ${d.path}\n  ${d.message}`);
  const count = diagnostics.length;
  return `${lines.join("\n\n")}\n\n${count} problem${count === 1 ? "" : "s"}`;
}

function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
}

/** Formats the cost report as a table, with each bound's share of its limit. */
export function formatReport(report: CheckReport): string {
  const limits = defaultLimits();
  const rows = Object.entries(report.routes).map(([id, { bounds }]) => [
    id,
    ...cells(bounds, {
      reads: limits.reads,
      writes: limits.writes,
      ops: limits.ops,
      bytesRead: limits.bytesRead,
    }),
  ]);
  const header = ["route", "reads", "writes", "ops", "bytes read"];
  const widths = header.map((h, i) => Math.max(h.length, ...rows.map((r) => r[i]?.length ?? 0)));
  const line = (cells: string[]) =>
    cells
      .map((c, i) => (i === 0 ? c.padEnd(widths[i] ?? 0) : c.padStart(widths[i] ?? 0)))
      .join("  ");
  return [line(header), ...rows.map(line)].join("\n");
}

function cells(bounds: Cost, limits: Cost): string[] {
  const pct = (value: number, limit: number) => `${Math.round((value / limit) * 100)}%`;
  return [
    `${bounds.reads} (${pct(bounds.reads, limits.reads)})`,
    `${bounds.writes} (${pct(bounds.writes, limits.writes)})`,
    `${bounds.ops} (${pct(bounds.ops, limits.ops)})`,
    `${bytes(bounds.bytesRead)} (${pct(bounds.bytesRead, limits.bytesRead)})`,
  ];
}

/** Validates a manifest. Returns the text to print and whether it's valid. */
export function check(manifestJson: string): { ok: boolean; output: string } {
  const result = validate(manifestJson);
  if (!result.ok) return { ok: false, output: formatDiagnostics(result.diagnostics) };
  return {
    ok: true,
    output: `✔ valid: worst-case cost per route (share of the route limit)\n\n${formatReport(result.report)}`,
  };
}
