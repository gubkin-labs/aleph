"use client";

import { type ReactNode, useEffect, useRef, useState } from "react";

/** One terminal line. `cmd` lines are typed out; the rest appear whole. */
export type Line = { t: string; k?: "cmd" | "agent" | "ok" | "err" | "dim" };

const TYPE_MS = 26;
const AFTER_CMD_MS = 450;
const LINE_MS = 90;

/**
 * A terminal panel that replays a session once it scrolls into view. Without JavaScript, or for
 * people who prefer reduced motion, the whole session is shown at once.
 */
export function Replay({
  title,
  caption,
  lines,
}: {
  title: string;
  caption: ReactNode;
  lines: Line[];
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [shown, setShown] = useState(lines.length);
  const [partial, setPartial] = useState<string | null>(null);
  const [run, setRun] = useState(0);

  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    let cancelled = false;
    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

    async function play() {
      setShown(0);
      for (let i = 0; i < lines.length; i++) {
        if (cancelled) return;
        const line = lines[i];
        if (!line) return;
        if (line.k === "cmd" || line.k === "agent") {
          for (let c = 1; c <= line.t.length; c++) {
            if (cancelled) return;
            setPartial(line.t.slice(0, c));
            await sleep(TYPE_MS);
          }
          setPartial(null);
          setShown(i + 1);
          await sleep(AFTER_CMD_MS);
        } else {
          setShown(i + 1);
          await sleep(LINE_MS);
        }
      }
    }

    setShown(0);
    if (run > 0) {
      // The replay button: the panel is already on screen.
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
      { threshold: 0.4 },
    );
    observer.observe(el);
    return () => {
      cancelled = true;
      observer.disconnect();
    };
  }, [lines, run]);

  const typing = partial !== null ? lines[shown] : null;

  return (
    <figure className="panel" ref={ref}>
      <figcaption className="panel-title">
        <span>{title}</span>
        <button
          type="button"
          className="panel-replay"
          onClick={() => setRun((n) => n + 1)}
          aria-label="Replay"
        >
          ↻
        </button>
      </figcaption>
      <pre className="panel-body" style={{ minHeight: `${lines.length * 1.6 + 2.4}em` }}>
        {lines.slice(0, shown).map((line, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: lines are static
          <div key={i} className={`ln ln-${line.k ?? "out"}`}>
            {line.t || " "}
          </div>
        ))}
        {typing && (
          <div className={`ln ln-${typing.k}`}>
            {partial}
            <span className="cursor" />
          </div>
        )}
      </pre>
      <div className="panel-caption">{caption}</div>
    </figure>
  );
}

/** A panel with a still or animated diagram instead of a terminal session. */
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
      <div className="panel-body panel-diagram">{children}</div>
      <div className="panel-caption">{caption}</div>
    </figure>
  );
}
