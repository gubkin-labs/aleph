/** `aleph check`: diagnostics, or a per-route cost table. */
import { defaultLimits, jobLimits, validate } from "@aleph/core";
import type { CheckReport, Cost, Diagnostic, Limits, RouteReport } from "@aleph/manifest";

/** Formats diagnostics for a terminal. */
export function formatDiagnostics(diagnostics: Diagnostic[]): string {
  const lines = diagnostics.map(
    (d) =>
      `✖ ${d.code} at ${d.location ?? d.path}${d.location ? ` (${d.path})` : ""}\n  ${d.message}`,
  );
  const count = diagnostics.length;
  return `${lines.join("\n\n")}\n\n${count} problem${count === 1 ? "" : "s"}`;
}

function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
}

function table(title: string, reports: Record<string, RouteReport>, limits: Limits): string {
  const header = [title, "reads", "writes", "ops", "bytes read", "effects", "calls"];
  const rows = Object.entries(reports).map(([id, { bounds }]) => [id, ...cells(bounds, limits)]);
  const widths = header.map((h, i) => Math.max(h.length, ...rows.map((r) => r[i]?.length ?? 0)));
  const line = (cells: string[]) =>
    cells
      .map((c, i) => (i === 0 ? c.padEnd(widths[i] ?? 0) : c.padStart(widths[i] ?? 0)))
      .join("  ");
  return [line(header), ...rows.map(line)].join("\n");
}

/** Formats the cost report: routes against the route limits, handlers against the job limits. */
export function formatReport(report: CheckReport): string {
  const parts = [table("route", report.routes, defaultLimits())];
  if (report.handlers && Object.keys(report.handlers).length > 0) {
    parts.push(table("handler", report.handlers, jobLimits()));
  }
  return parts.join("\n\n");
}

function cells(bounds: Cost, limits: Limits): string[] {
  const pct = (value: number, limit: number) => `${Math.round((value / limit) * 100)}%`;
  return [
    `${bounds.reads} (${pct(bounds.reads, limits.reads)})`,
    `${bounds.writes} (${pct(bounds.writes, limits.writes)})`,
    `${bounds.ops} (${pct(bounds.ops, limits.ops)})`,
    `${bytes(bounds.bytesRead)} (${pct(bounds.bytesRead, limits.bytesRead)})`,
    `${bounds.effects} (${pct(bounds.effects, limits.effects)})`,
    `${bounds.calls} (${pct(bounds.calls, limits.calls)})`,
  ];
}

/** Validates a manifest. Returns the text to print and whether it's valid. */
export function check(manifestJson: string): { ok: boolean; output: string } {
  const result = validate(manifestJson);
  if (!result.ok) return { ok: false, output: formatDiagnostics(result.diagnostics) };
  return {
    ok: true,
    output: `✔ valid: worst-case cost (share of the limit)\n\n${formatReport(result.report)}`,
  };
}
