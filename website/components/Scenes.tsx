/**
 * The homepage's conceptual diagrams: SVGs animated with CSS (see globals.css, "Scenes"). Each
 * element's resting style is its final state, so without motion every scene is a complete still.
 */

/** Many apps share one runtime; each request stays under its own ceiling. */
export function SharedScene() {
  const tiles = Array.from({ length: 24 }, (_, i) => i);
  return (
    <svg
      viewBox="0 0 560 300"
      className="scene"
      role="img"
      aria-label="Three separate stacks sit mostly idle, while one shared runtime holds two dozen apps, each request under its own ceiling"
    >
      <text x="16" y="26" className="s-label">
        a stack per app
      </text>
      {[0, 1, 2].map((i) => (
        <g key={i} transform={`translate(16, ${44 + i * 76})`}>
          <rect width="220" height="64" rx="6" className="s-card" />
          <text x="12" y="23" className="s-tiny">
            server · db · cache
          </text>
          <rect x="12" y="36" width="196" height="10" rx="3" className="s-track" />
          <rect
            x="12"
            y="36"
            width="196"
            height="10"
            rx="3"
            className="s-idle"
            style={{ animationDelay: `${i * -1.3}s` }}
          />
          <text x="208" y="23" className="s-tiny s-muted" textAnchor="end">
            idle
          </text>
        </g>
      ))}
      <text x="16" y="276" className="s-tiny s-muted">
        each sized for its peak,
      </text>
      <text x="16" y="292" className="s-tiny s-muted">
        paid for around the clock
      </text>

      <text x="260" y="26" className="s-label">
        one shared runtime
      </text>
      <rect x="260" y="44" width="284" height="216" rx="6" className="s-card" />
      {tiles.map((i) => {
        const x = 274 + (i % 6) * 44;
        const y = 58 + Math.floor(i / 6) * 49;
        const dur = 1.8 + ((i * 7) % 5) * 0.35;
        return (
          <g key={i} className="s-tile" style={{ animationDelay: `${i * 0.11}s` }}>
            <rect x={x} y={y} width="38" height="41" rx="4" className="s-app" />
            <line x1={x + 6} y1={y + 9} x2={x + 32} y2={y + 9} className="s-ceiling" />
            <rect
              x={x + 13}
              y={y + 9}
              width="12"
              height="26"
              rx="2"
              className="s-req"
              style={{ animationDuration: `${dur}s`, animationDelay: `${(i * 0.37) % 2}s` }}
            />
          </g>
        );
      })}
      <rect x="260" y="272" width="284" height="8" rx="3" className="s-track" />
      <rect x="260" y="272" width="284" height="8" rx="3" className="s-meter" />
      <text x="544" y="294" className="s-tiny s-muted" textAnchor="end">
        pooled capacity, used
      </text>
    </svg>
  );
}

/** Every key is built inside its tenant's keyspace; another tenant's keys can't be named. */
export function IsolationScene() {
  const tenants = [
    { name: "shop", x: 20, cx: 100 },
    { name: "crm", x: 200, cx: 280 },
    { name: "billing", x: 380, cx: 460 },
  ];
  return (
    <svg
      viewBox="0 0 560 260"
      className="scene"
      role="img"
      aria-label="Requests from three apps each land in their own range of one shared database; a key in another app's range can't be named"
    >
      <rect x="16" y="160" width="528" height="54" rx="6" className="s-card" />
      {tenants.map((t, i) => (
        <g key={t.name}>
          <rect x={t.cx - 46} y="16" width="92" height="28" rx="14" className="s-pill" />
          <text x={t.cx} y="34" className="s-tiny" textAnchor="middle">
            app: {t.name}
          </text>
          <path d={`M${t.cx} 44 L${t.cx} 170`} className="s-edge" />
          <path
            d={`M${t.cx} 44 L${t.cx} 170`}
            pathLength={1}
            className="s-drop"
            style={{ animationDelay: `${i * 1.1}s` }}
          />
          <rect x={t.x + 4} y="166" width="152" height="42" rx="4" className="s-range" />
          {[0, 1, 2, 3, 4, 5, 6, 7, 8, 9].map((k) => (
            <line
              key={k}
              x1={t.x + 16 + k * 14}
              y1="198"
              x2={t.x + 16 + k * 14}
              y2="203"
              className="s-key"
            />
          ))}
          <circle
            cx={t.cx}
            cy="187"
            r="6"
            className="s-hit"
            style={{ animationDelay: `${i * 1.1}s` }}
          />
          <text x={t.x + 12} y="232" className="s-tiny s-muted">
            {t.name}/Note/…
          </text>
        </g>
      ))}
      <path d="M126 44 C 150 80, 196 92, 214 112" className="s-blocked" />
      <g className="s-cross">
        <line x1="210" y1="108" x2="222" y2="120" />
        <line x1="222" y1="108" x2="210" y2="120" />
      </g>
      <text x="200" y="140" className="s-tiny s-err-text" textAnchor="middle">
        no expression names
      </text>
      <text x="200" y="154" className="s-tiny s-err-text" textAnchor="middle">
        another app's keys
      </text>
      <text x="16" y="254" className="s-tiny s-muted">
        one shared, ordered keyspace · each app reaches only its own prefix
      </text>
    </svg>
  );
}

/** One description in, everything a backend needs derived from it. */
export function GeneratedScene() {
  const outputs = [
    "runs on any Aleph runtime",
    "OpenAPI spec",
    "typed client",
    "MCP tools",
    "tests from examples",
  ];
  const gone = ["Dockerfile", "CI pipeline", "ORM", "queue client", "auth library"];
  return (
    <svg
      viewBox="0 0 560 280"
      className="scene"
      role="img"
      aria-label="app.ts compiles to a manifest, which runs on any Aleph runtime and produces an OpenAPI spec, a typed client, MCP tools, and tests"
    >
      <rect x="16" y="92" width="104" height="48" rx="6" className="s-card s-strong-card" />
      <text x="68" y="121" className="s-label" textAnchor="middle">
        app.ts
      </text>
      <path d="M120 116 L160 116" className="s-edge" />
      <path d="M120 116 L160 116" pathLength={1} className="s-flow" />
      <rect x="160" y="92" width="136" height="48" rx="6" className="s-card" />
      <text x="228" y="113" className="s-label" textAnchor="middle">
        manifest.json
      </text>
      <text x="228" y="129" className="s-tiny s-muted" textAnchor="middle">
        data, not code
      </text>
      {outputs.map((name, i) => {
        const y = 20 + i * 44;
        const d = `M296 116 C 326 116, 326 ${y + 15}, 356 ${y + 15}`;
        return (
          <g key={name}>
            <path d={d} className="s-edge" />
            <path
              d={d}
              pathLength={1}
              className="s-flow"
              style={{ animationDelay: `${0.6 + i * 0.25}s` }}
            />
            <rect
              x="356"
              y={y}
              width="188"
              height="30"
              rx="6"
              className="s-card s-out"
              style={{ animationDelay: `${0.6 + i * 0.25}s` }}
            />
            <text x="370" y={y + 19} className="s-tiny">
              {name}
            </text>
          </g>
        );
      })}
      <text x="16" y="196" className="s-tiny s-muted">
        not needed:
      </text>
      {gone.map((name, i) => {
        const x = 16 + (i % 3) * 96;
        const y = 220 + Math.floor(i / 3) * 24;
        const w = name.length * 6.6;
        return (
          <g key={name}>
            <text x={x} y={y} className="s-tiny s-gone">
              {name}
            </text>
            <line
              x1={x - 2}
              y1={y - 4}
              x2={x + w + 2}
              y2={y - 4}
              pathLength={1}
              className="s-strike"
              style={{ animationDelay: `${2 + i * 0.3}s` }}
            />
          </g>
        );
      })}
    </svg>
  );
}
