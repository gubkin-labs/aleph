"use client";

import { type ReactNode, useEffect, useRef, useState } from "react";

/** Holds a scene's animations until it is on screen, so people see it from the start. */
export function Visible({ children }: { children: ReactNode }) {
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
      { threshold: 0.6 },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return (
    <div ref={ref} className={seen ? "pg-view in" : "pg-view"}>
      {children}
    </div>
  );
}
