/** `aleph check`: diagnostics, or a table of each route's worst-case usage. */
import { defaultLimits, type Evolution, jobLimits, validate } from "@aleph-lang/core";
import type { CheckReport, Cost, Diagnostic, Limits, RouteReport } from "@aleph-lang/manifest";

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
  const mb = n / 1024 / 1024;
  return `${Number.isInteger(mb) ? mb : mb.toFixed(2)} MB`;
}

/** A value's share of its limit: `–` for none, `<1%` for a little, never a misleading `0%`. */
function share(value: number, limit: number): string {
  if (value === 0) return "–";
  const pct = (value / limit) * 100;
  return pct < 1 ? "<1%" : `${Math.round(pct)}%`;
}

type Resource = keyof Cost & keyof Limits;

const COLUMNS: { name: string; key: Resource; format: (n: number) => string }[] = [
  { name: "reads", key: "reads", format: String },
  { name: "writes", key: "writes", format: String },
  { name: "ops", key: "ops", format: String },
  { name: "bytes read", key: "bytesRead", format: bytes },
  { name: "effects", key: "effects", format: String },
  { name: "calls", key: "calls", format: String },
  { name: "memory", key: "memory", format: bytes },
];

/** A row's name, and each column's value and share of the limit. */
type Row = { name: string; cells: { value: string; share: string }[] };

const grouped = (s: string) => s.replace(/\B(?=(\d{3})+(?!\d))/g, ",");

/**
 * One table: a limit row, then each route's (or handler's) worst-case usage in one run, as a value
 * and its share of the limit.
 */
function table(title: string, reports: Record<string, RouteReport>, limits: Limits): string {
  const limitRow: Row = {
    name: "limit",
    cells: COLUMNS.map((c) => ({ value: grouped(c.format(limits[c.key])), share: "" })),
  };
  const rows: Row[] = Object.entries(reports).map(([id, { bounds }]) => ({
    name: id,
    cells: COLUMNS.map((c) => ({
      value: c.format(bounds[c.key]),
      share: share(bounds[c.key], limits[c.key]),
    })),
  }));
  const all = [limitRow, ...rows];
  const nameWidth = Math.max(title.length, ...all.map((r) => r.name.length));
  const widths = COLUMNS.map((c, i) => {
    const value = Math.max(...all.map((r) => r.cells[i]?.value.length ?? 0));
    const pct = Math.max(...all.map((r) => r.cells[i]?.share.length ?? 0));
    return { value, pct, cell: Math.max(c.name.length, value + 1 + pct) };
  });
  const line = (row: Row) =>
    [
      row.name.padEnd(nameWidth),
      ...row.cells.map((cell, i) => {
        const w = widths[i] ?? { value: 0, pct: 0, cell: 0 };
        return `${cell.value.padStart(w.value)} ${cell.share.padStart(w.pct)}`.padStart(w.cell);
      }),
    ]
      .join("   ")
      .trimEnd();
  const header = [
    title.padEnd(nameWidth),
    ...COLUMNS.map((c, i) => c.name.padStart(widths[i]?.cell ?? 0)),
  ].join("   ");
  return [header, line(limitRow), ...rows.map(line)].join("\n");
}

/** Formats the usage report: routes against the route limits, handlers against the job limits. */
export function formatReport(report: CheckReport): string {
  const parts = [table("route", report.routes, defaultLimits())];
  if (report.handlers && Object.keys(report.handlers).length > 0) {
    parts.push(table("handler", report.handlers, jobLimits()));
  }
  return parts.join("\n\n");
}

/** The line above the usage report. */
export const REPORT_HEADING = "✔ valid. The most each route and handler can use in one run:";

/** Formats a schema evolution report. */
export function formatEvolution(evolution: Evolution): string {
  const mark = { safe: "✔", needsBackfill: "↻", breaking: "✖" } as const;
  const lines = evolution.changes.map((c) => {
    const allowed = c.kind === "breaking" && c.allowed ? " (allowed: deprecated first)" : "";
    return `${mark[c.kind]} ${c.path}: ${c.message}${allowed}`;
  });
  const tasks = evolution.migration.map((t) =>
    t.task === "buildIndex" || t.task === "dropIndex"
      ? `  ${t.task} ${t.entity}.${t.index}`
      : `  ${t.task} ${t.counter}`,
  );
  return [
    evolution.changes.length === 0 ? "no changes from the deployed manifest" : lines.join("\n"),
    ...(tasks.length > 0 ? ["", "migration plan:", ...tasks] : []),
    "",
    evolution.deployable
      ? "✔ deployable"
      : "✖ not deployable: breaking changes need a deprecation in the deployed manifest first",
  ].join("\n");
}

/** Validates a manifest. Returns the text to print and whether it's valid. */
export function check(manifestJson: string): { ok: boolean; output: string } {
  const result = validate(manifestJson);
  if (!result.ok) return { ok: false, output: formatDiagnostics(result.diagnostics) };
  return {
    ok: true,
    output: `${REPORT_HEADING}\n\n${formatReport(result.report)}`,
  };
}
