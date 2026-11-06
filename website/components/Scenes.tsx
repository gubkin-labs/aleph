/**
 * The homepage's diagram of shared infrastructure: a small SVG animated with CSS (see
 * globals.css, "Scenes"). Motion stops for people who prefer reduced motion.
 */

/** Shared resources: many apps, one runtime and one database, instead of a stack each. */
export function SharedScene() {
  const apps = Array.from({ length: 18 }, (_, i) => i);
  return (
    <svg
      viewBox="0 0 520 260"
      className="scene"
      role="img"
      aria-label="Many small apps share one runtime instead of each running mostly idle infrastructure"
    >
      <text x="20" y="34" className="s-label">
        a stack per app
      </text>
      {[0, 1, 2, 3].map((i) => (
        <g key={i} transform={`translate(${20 + (i % 2) * 120}, ${48 + Math.floor(i / 2) * 92})`}>
          <rect width="112" height="78" rx="8" className="s-card" />
          <rect x="10" y="56" width="92" height="10" rx="3" className="s-track" />
          <rect x="10" y="56" width="12" height="10" rx="3" className="s-used s-idle" />
          <text x="10" y="24" className="s-tiny">
            server · db
          </text>
          <text x="10" y="44" className="s-tiny s-muted">
            mostly idle
          </text>
        </g>
      ))}
      <text x="270" y="34" className="s-label">
        shared by many apps
      </text>
      <rect x="270" y="48" width="230" height="170" rx="10" className="s-card" />
      <text x="284" y="70" className="s-tiny">
        one runtime · one database
      </text>
      {apps.map((i) => (
        <rect
          key={i}
          x={286 + (i % 6) * 35}
          y={84 + Math.floor(i / 6) * 40}
          width="28"
          height="30"
          rx="5"
          className={`s-app s-app-${i % 4}`}
          style={{ animationDelay: `${i * 0.18}s` }}
        />
      ))}
    </svg>
  );
}
