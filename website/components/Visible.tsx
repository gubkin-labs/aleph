"use client";

import { type ReactNode, useEffect, useRef, useState } from "react";

/** Holds a scene's animations until it is on screen, so people see it from the start. */
export function Visible({
  children,
  threshold = 0.6,
}: {
  children: ReactNode;
  /** How much of it must be on screen to start; tall scenes need a small share. */
  threshold?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setSeen(true);
          observer.disconnect();
        }
      },
      { threshold },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [threshold]);
  return (
    <div ref={ref} className={seen ? "pg-view in" : "pg-view"}>
      {children}
    </div>
  );
}
