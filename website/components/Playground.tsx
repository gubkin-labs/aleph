"use client";

import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from "react";

/**
 * The homepage playground: tabs along the top, each showing one advantage as Aleph code beside
 * what the same thing takes in a typical Node backend. Tabs advance on a timer, shown as a bar
 * under the active tab. The timer waits until the playground is on screen, pauses while the
 * pointer or focus is inside it or it scrolls away, and stops for good once someone picks a tab.
 * For people who prefer reduced motion there is no timer: just the tabs, in their final state.
 */
export type PlaygroundTab = {
  id: string;
  label: string;
  /** How long the tab stays up before the next one, in milliseconds. */
  ms: number;
  /** "code" is a scene over each code pane; "stack" is two scenes, one over the other; "single" is one scene for the whole tab. */
  layout: "code" | "stack" | "single";
  /** The tab's cells, each placed in the grid by its own `pg-c*` and `pg-r*` classes. */
  body: ReactNode;
};

export function Playground({ tabs }: { tabs: PlaygroundTab[] }) {
  const frame = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState(0);
  const [replay, setReplay] = useState(0);
  const [auto, setAuto] = useState(true);
  const [onScreen, setOnScreen] = useState(false);
  const [hover, setHover] = useState(false);

  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) setAuto(false);
    const el = frame.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      (entries) => setOnScreen(entries.some((e) => e.isIntersecting)),
      { threshold: 0.3 },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const tab = tabs[active] as PlaygroundTab;
  const select = (i: number) => {
    setActive(i);
    setReplay((n) => n + 1);
    setAuto(false);
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const next = (active + (e.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length;
    select(next);
    document.getElementById(`pg-tab-${tabs[next]?.id}`)?.focus();
  };

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: hover and focus only pause the timer
    <div
      className="pg"
      ref={frame}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onFocus={() => setHover(true)}
      onBlur={() => setHover(false)}
    >
      <div
        className="pg-tabs"
        role="tablist"
        aria-label="What Aleph rules out"
        onKeyDown={onKeyDown}
      >
        {tabs.map((t, i) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            id={`pg-tab-${t.id}`}
            aria-selected={i === active}
            aria-controls={`pg-panel-${t.id}`}
            tabIndex={i === active ? 0 : -1}
            className={i === active ? "pg-tab active" : "pg-tab"}
            onClick={() => select(i)}
          >
            <span>{t.label}</span>
            {i === active && auto && (
              <i
                className="pg-bar"
                aria-hidden
                // The bar is the timer: when it fills, the next tab opens.
                key={`${t.id}-${replay}`}
                style={{
                  animationDuration: `${t.ms}ms`,
                  animationPlayState: onScreen && !hover ? "running" : "paused",
                }}
                onAnimationEnd={() => {
                  setActive((active + 1) % tabs.length);
                  setReplay((n) => n + 1);
                }}
              />
            )}
          </button>
        ))}
        <button
          type="button"
          className="pg-replay"
          onClick={() => setReplay((n) => n + 1)}
          aria-label="Replay this example"
          title="Replay"
        >
          ↻
        </button>
      </div>

      <div
        className={`pg-panes pg-${tab.layout}`}
        role="tabpanel"
        id={`pg-panel-${tab.id}`}
        aria-labelledby={`pg-tab-${tab.id}`}
        key={`${tab.id}-${replay}`}
      >
        {tab.body}
      </div>
    </div>
  );
}
