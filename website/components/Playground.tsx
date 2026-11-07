"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { Category, Item, Option } from "@/app/playgrounds";
import { highlight } from "./Demo";

const STEP_MS = 420;

/**
 * Small playgrounds in the spirit of effect.website: pick a feature, change one value, and watch
 * what Aleph does with it. The visuals replay real `aleph check` and `aleph dev` results.
 */
export function Playground({ categories }: { categories: Category[] }) {
  const [cat, setCat] = useState(0);
  const [ex, setEx] = useState(0);
  const [opt, setOpt] = useState(0);
  const [phase, setPhase] = useState(Number.POSITIVE_INFINITY);
  const [run, setRun] = useState(0);
  const ref = useRef<HTMLElement>(null);

  const category = categories[cat];
  const example = category?.examples[ex];
  const option = example?.options[opt];
  const total = option?.seq.length ?? 0;

  const play = useCallback(() => setRun((n) => n + 1), []);

  // Start once the playground scrolls into view.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
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
    return () => observer.disconnect();
  }, [play]);

  useEffect(() => {
    if (run === 0) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      setPhase(Number.POSITIVE_INFINITY);
      return;
    }
    setPhase(-1);
    let i = -1;
    const timer = setInterval(() => {
      i += 1;
      setPhase(i);
      if (i >= total) clearInterval(timer);
    }, STEP_MS);
    return () => clearInterval(timer);
  }, [run, total]);

  const choose = (c: number, e: number, o: number) => {
    setCat(c);
    setEx(e);
    setOpt(o);
    play();
  };

  if (!category || !example || !option) return null;

  return (
    <section className="playground" ref={ref} aria-label="Playground">
      <div className="pg-cats" role="tablist">
        {categories.map((c, i) => (
          <button
            key={c.name}
            type="button"
            role="tab"
            aria-selected={i === cat}
            className={i === cat ? "active" : undefined}
            onClick={() => choose(i, 0, 0)}
          >
            {c.name}
          </button>
        ))}
      </div>
      <div className="pg-examples" role="tablist">
        {category.examples.map((e, i) => (
          <button
            key={e.name}
            type="button"
            role="tab"
            aria-selected={i === ex}
            className={i === ex ? "active" : undefined}
            onClick={() => choose(cat, i, 0)}
          >
            {e.name}
          </button>
        ))}
      </div>

      <div className="pg-card">
        <div className="pg-head">
          <button type="button" className="pg-play" onClick={play} aria-label="Run">
            ▶
          </button>
          <div>
            <div className="pg-name">{example.name}</div>
            <div className="pg-desc">{example.description}</div>
          </div>
        </div>

        <div className="pg-control">
          <span>{example.control}</span>
          <div className="pg-options">
            {example.options.map((o, i) => (
              <button
                key={o.label}
                type="button"
                className={i === opt ? "active" : undefined}
                onClick={() => choose(cat, ex, i)}
              >
                {o.label}
              </button>
            ))}
          </div>
        </div>

        <div className="pg-visual">
          {option.seq.map((item, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: items are positional within an option
            <Visual key={`${run}-${i}`} item={item} on={i <= phase} />
          ))}
        </div>

        <Code option={option} />
      </div>
    </section>
  );
}

function Visual({ item, on }: { item: Item; on: boolean }) {
  if ("op" in item) return <span className={on ? "pg-op on" : "pg-op"}>{item.op}</span>;
  if ("meter" in item) {
    const share = Math.min(1, item.value / item.max);
    const over = item.value > item.max;
    return (
      <div className={["pg-meter", on && "on", over && "over"].filter(Boolean).join(" ")}>
        <div className="pg-meter-bar">
          <div className="pg-meter-fill" style={{ transform: `scaleX(${on ? share : 0})` }} />
        </div>
        <div className="pg-meter-label">
          {item.meter} {item.value.toLocaleString("en-US")} / {item.max.toLocaleString("en-US")}
        </div>
      </div>
    );
  }
  if ("result" in item)
    return (
      <div className={["pg-result", `pg-${item.tone}`, on && "on"].filter(Boolean).join(" ")}>
        {item.result}
      </div>
    );
  return (
    <div
      className={["pg-box", item.tone && `pg-${item.tone}`, on && "on"].filter(Boolean).join(" ")}
    >
      <div className="pg-box-main">{item.box}</div>
      {item.sub && <div className="pg-box-sub">{item.sub}</div>}
    </div>
  );
}

function Code({ option }: { option: Option }) {
  return (
    <pre className="pg-code">
      {option.code.map((line, i) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional
          key={i}
          className={option.hl?.includes(i + 1) ? "pg-line pg-line-hl" : "pg-line"}
        >
          {highlight(line)}
          {line === "" ? " " : null}
        </div>
      ))}
    </pre>
  );
}
