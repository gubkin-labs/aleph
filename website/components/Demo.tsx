"use client";

import { Fragment, type ReactNode, useEffect, useRef, useState } from "react";

/**
 * A homepage demo: an editor showing a slice of app.ts above a terminal, driven by a short
 * script. It plays once when scrolled into view and can be replayed. Without JavaScript, or for
 * people who prefer reduced motion, it shows a still frame (by default the end of the script).
 */

export type Tone = "ok" | "err";
/** `table` lines never wrap (they scroll instead), so columns stay aligned. */
export type Line = {
  t: string;
  k?: "cmd" | "agent" | "ok" | "err" | "dim";
  hl?: boolean;
  table?: boolean;
};

export type Step =
  | { cmd: string }
  | { agent: string }
  | { out: Line[] }
  | { hl: number[]; tone?: Tone; mark?: string | string[] }
  | { termHl: number[] }
  | { edit: number; from: string; to: string }
  | { insert: number; text: string }
  | { clear: true }
  | { pause: number };

type CodeLine = { text: string; orig: string | null };
type Typing = { where: "term"; line: Line } | { where: "code"; index: number; pos: number } | null;
type State = {
  code: CodeLine[];
  hl: number[];
  tone: Tone | null;
  mark: string[];
  term: Line[];
  typing: Typing;
};

const SPEED = { cmd: 24, agent: 14, codeType: 65, codeErase: 40, out: 60, afterCmd: 450 };

function initial(code: string[]): State {
  return {
    code: code.map((text) => ({ text, orig: text })),
    hl: [],
    tone: null,
    mark: [],
    term: [],
    typing: null,
  };
}

/** Line numbers are positions, so `hl`, `edit`, and `insert` use the editor's current numbering. */
function indexOf(start: number, n: number) {
  return n - start;
}

function apply(s: State, step: Step, start: number): State {
  if ("cmd" in step) return { ...s, term: [...s.term, { t: step.cmd, k: "cmd" }], typing: null };
  if ("agent" in step)
    return { ...s, term: [...s.term, { t: step.agent, k: "agent" }], typing: null };
  if ("out" in step) return { ...s, term: [...s.term, ...step.out] };
  if ("hl" in step)
    return { ...s, hl: step.hl, tone: step.tone ?? null, mark: [step.mark ?? []].flat() };
  if ("termHl" in step)
    return { ...s, term: s.term.map((l, i) => (step.termHl.includes(i) ? { ...l, hl: true } : l)) };
  if ("clear" in step) return { ...s, term: [] };
  if ("edit" in step) {
    const i = indexOf(start, step.edit);
    const code = s.code.map((l, j) =>
      j === i ? { ...l, text: l.text.replace(step.from, step.to) } : l,
    );
    return { ...s, code, typing: null };
  }
  if ("insert" in step) {
    const i = indexOf(start, step.insert);
    const code = [
      ...s.code.slice(0, i + 1),
      { text: step.text, orig: null },
      ...s.code.slice(i + 1),
    ];
    return { ...s, code, typing: null };
  }
  return s;
}

/** Rough rendered height of terminal lines, counting long lines that wrap. */
function weight(lines: Line[]) {
  return lines.reduce((n, l) => n + 1 + Math.floor(l.t.length / 72), 0);
}

/**
 * Runs the script without animation: the still frame, plus the fullest editor and terminal, which
 * are rendered invisibly so the panel never changes size while it plays.
 */
function simulate(code: string[], steps: Step[], start: number, upTo = steps.length) {
  let s = initial(code);
  let cur = s;
  let maxCode = s.code;
  let maxTerm: Line[] = [];
  for (const [i, step] of steps.entries()) {
    cur = apply(cur, step, start);
    if (cur.code.length > maxCode.length) maxCode = cur.code;
    if (weight(cur.term) > weight(maxTerm)) maxTerm = cur.term;
    if (i < upTo) s = cur;
  }
  return { still: s, maxCode, maxTerm };
}

// ─── Syntax colors: a tiny tokenizer, enough for SDK code ───────────────────────

const TOKEN =
  /(\/\/.*$)|("(?:[^"\\]|\\.)*"?|'(?:[^'\\]|\\.)*'?|`[^`]*`?)|(\b\d+\b)|\b(import|from|const|export|default|return|true|false)\b|(\.\w+)(?=\()|(\b[A-Z]\w*\b)/g;

export function highlight(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  for (const m of text.matchAll(TOKEN)) {
    const at = m.index ?? 0;
    if (at > last) out.push(text.slice(last, at));
    const cls = m[1] ? "c" : m[2] ? "s" : m[3] ? "n" : m[4] ? "k" : m[5] ? "f" : "ty";
    out.push(
      <span key={at} className={`tk-${cls}`}>
        {m[0]}
      </span>,
    );
    last = at + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

function lineClass(line: Line) {
  return ["ln", `ln-${line.k ?? "out"}`, line.hl && "ln-hl", line.table && "ln-table"]
    .filter(Boolean)
    .join(" ");
}

// ─── Component ──────────────────────────────────────────────────────────────────

export function Demo({
  title,
  file = "app.ts",
  start,
  code,
  steps,
  still,
  rows,
  caption,
}: {
  title: string;
  file?: string;
  /** The real line number of the first line shown. */
  start: number;
  code: string[];
  steps: Step[];
  /** How many steps make up the still frame (default: all of them). */
  still?: number;
  /** Show at most this many code lines, scrolling to keep the highlight in view. */
  rows?: number;
  caption: ReactNode;
}) {
  const ref = useRef<HTMLElement>(null);
  const sim = simulate(code, steps, start, still);
  const [state, setState] = useState<State>(sim.still);
  const [playing, setPlaying] = useState(false);
  const [run, setRun] = useState(0);

  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    let cancelled = false;
    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

    async function play() {
      setPlaying(true);
      let s = initial(code);
      const set = (next: State) => {
        s = next;
        if (!cancelled) setState(next);
      };
      set(s);
      await sleep(500);
      for (const step of steps) {
        if (cancelled) return;
        if ("cmd" in step || "agent" in step) {
          const kind = "cmd" in step ? "cmd" : "agent";
          const text = "cmd" in step ? step.cmd : step.agent;
          for (let c = 1; c <= text.length; c++) {
            if (cancelled) return;
            set({ ...s, typing: { where: "term", line: { t: text.slice(0, c), k: kind } } });
            await sleep(kind === "cmd" ? SPEED.cmd : SPEED.agent);
          }
          set(apply(s, step, start));
          await sleep(SPEED.afterCmd);
        } else if ("out" in step) {
          for (const line of step.out) {
            if (cancelled) return;
            set({ ...s, term: [...s.term, line] });
            await sleep(SPEED.out);
          }
        } else if ("edit" in step) {
          const i = indexOf(start, step.edit);
          const line = s.code[i];
          if (!line) continue;
          const at = line.text.indexOf(step.from);
          if (at < 0) continue;
          const before = line.text.slice(0, at);
          const after = line.text.slice(at + step.from.length);
          const put = (mid: string) =>
            set({
              ...s,
              code: s.code.map((l, j) => (j === i ? { ...l, text: before + mid + after } : l)),
              typing: { where: "code", index: i, pos: before.length + mid.length },
            });
          for (let c = step.from.length; c >= 0; c--) {
            if (cancelled) return;
            put(step.from.slice(0, c));
            await sleep(SPEED.codeErase);
          }
          for (let c = 1; c <= step.to.length; c++) {
            if (cancelled) return;
            put(step.to.slice(0, c));
            await sleep(SPEED.codeType);
          }
          await sleep(250);
          set({ ...s, typing: null });
        } else if ("insert" in step) {
          const i = indexOf(start, step.insert) + 1;
          const indent = step.text.match(/^\s*/)?.[0] ?? "";
          const body = step.text.slice(indent.length);
          const put = (text: string) => {
            const code = [...s.code];
            if (code[i]?.orig === null && code[i]?.text.startsWith(indent))
              code[i] = { text, orig: null };
            else code.splice(i, 0, { text, orig: null });
            set({ ...s, code, typing: { where: "code", index: i, pos: text.length } });
          };
          put(indent);
          for (let c = 1; c <= body.length; c++) {
            if (cancelled) return;
            put(indent + body.slice(0, c));
            await sleep(SPEED.codeType / 2);
          }
          await sleep(250);
          set({ ...s, typing: null });
        } else if ("pause" in step) {
          await sleep(step.pause);
        } else {
          set(apply(s, step, start));
          await sleep(120);
        }
      }
      if (!cancelled) setPlaying(false);
    }

    if (run > 0) {
      play();
      return () => {
        cancelled = true;
      };
    }
    const el = ref.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          observer.disconnect();
          play();
        }
      },
      { threshold: 0.55 },
    );
    observer.observe(el);
    return () => {
      cancelled = true;
      observer.disconnect();
    };
  }, [code, steps, start, run]);

  // On narrow screens, scroll the editor sideways to keep the caret or highlight in view.
  useEffect(() => {
    const body = ref.current?.querySelector<HTMLElement>(".editor-body");
    const target = body?.querySelector<HTMLElement>(".caret, .tok-mark");
    if (!body || !target) return;
    const b = body.getBoundingClientRect();
    const t = target.getBoundingClientRect();
    if (t.right > b.right - 12) body.scrollLeft += t.right - b.right + 48;
    else if (t.left < b.left + 40) body.scrollLeft -= b.left + 40 - t.left + 24;
  });

  // In a scrolling editor, bring the highlighted lines into view.
  const lit = state.hl.join(",");
  useEffect(() => {
    const body = ref.current?.querySelector<HTMLElement>(".editor-body");
    if (!rows || !body || !lit) return;
    const first = body.querySelector<HTMLElement>(".lit");
    if (!first) return;
    const top = first.offsetTop - body.clientHeight / 3;
    body.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
  }, [lit, rows]);

  const typing = state.typing;
  const width = String(start + sim.maxCode.length).length;

  return (
    <figure className="demo" ref={ref}>
      <figcaption className="panel-title">
        <span>{title}</span>
        <button
          type="button"
          className={playing ? "panel-replay" : "panel-replay ready"}
          onClick={() => setRun((n) => n + 1)}
          aria-label="Replay"
          title="Replay"
        >
          ↻
        </button>
      </figcaption>

      <div className="editor">
        <div className="editor-tab">{file}</div>
        <div
          className={rows ? "editor-body stack editor-scroll" : "editor-body stack"}
          style={rows ? { maxHeight: `calc(${rows} * 11.5px * 1.7 + 8px)` } : undefined}
        >
          <div className="ghost" aria-hidden>
            {sim.maxCode.map((line, i) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional
              <div key={i} className="code-line">
                <span className="gutter">{String(start + i).padStart(width, " ")}</span>
                <span className="code-text">{line.text || " "}</span>
              </div>
            ))}
          </div>
          <div>
            {state.code.map((line, i) => {
              const n = start + i;
              const lit = state.hl.includes(n);
              const mark = line.orig === null ? "add" : line.text !== line.orig ? "mod" : "";
              const cls = [
                "code-line",
                lit && `lit lit-${state.tone ?? "on"}`,
                mark && `mark-${mark}`,
              ]
                .filter(Boolean)
                .join(" ");
              const caret = typing?.where === "code" && typing.index === i ? typing.pos : -1;
              const tok = lit ? state.mark.find((m) => line.text.includes(m)) : undefined;
              return (
                // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional
                <div key={i} className={cls}>
                  <span className="gutter">{String(n).padStart(width, " ")}</span>
                  <span className="code-text">
                    {caret < 0 ? (
                      tok ? (
                        <Fragment>
                          {highlight(line.text.slice(0, line.text.indexOf(tok)))}
                          <mark className={`tok-mark tok-mark-${state.tone ?? "on"}`}>
                            {highlight(tok)}
                          </mark>
                          {highlight(line.text.slice(line.text.indexOf(tok) + tok.length))}
                        </Fragment>
                      ) : (
                        highlight(line.text)
                      )
                    ) : (
                      <Fragment>
                        {highlight(line.text.slice(0, caret))}
                        <span className="caret" />
                        {highlight(line.text.slice(caret))}
                      </Fragment>
                    )}
                    {line.text === "" && caret < 0 ? " " : null}
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      </div>

      <div className="term stack">
        <div className="ghost" aria-hidden>
          {sim.maxTerm.map((line, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional
            <div key={i} className={line.table ? "ln ln-table" : "ln"}>
              {line.t || " "}
            </div>
          ))}
          <div className="ln"> </div>
        </div>
        <div>
          {state.term.map((line, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional
            <div key={i} className={lineClass(line)}>
              {line.t || " "}
            </div>
          ))}
          {typing?.where === "term" && (
            <div className={`ln ln-${typing.line.k}`}>
              {typing.line.t}
              <span className="cursor" />
            </div>
          )}
        </div>
      </div>

      <div className="panel-caption">{caption}</div>
    </figure>
  );
}

/** A panel holding a conceptual diagram instead of an editor and terminal. */
export function Panel({
  title,
  caption,
  children,
}: {
  title: string;
  caption: ReactNode;
  children: ReactNode;
}) {
  return (
    <figure className="panel">
      <figcaption className="panel-title">
        <span>{title}</span>
      </figcaption>
      <div className="panel-diagram">{children}</div>
      <div className="panel-caption">{caption}</div>
    </figure>
  );
}
