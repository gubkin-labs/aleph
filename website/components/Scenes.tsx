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

/** Two cars, two roads: one weaves through everything a backend needs, the other gets it built in. */
export function TradeoffScene() {
  const obstacles = [
    { t: ["CPU / memory", "alert"], x: 110, up: true },
    { t: ["DB load"], x: 190, up: false },
    { t: ["tests, CI,", "deploys"], x: 270, up: true },
    { t: ["slow", "responses"], x: 350, up: false },
    { t: ["huge bill"], x: 430, up: true },
  ];
  const features = [
    { t: "auth", x: 80, row: 0 },
    { t: "transactions", x: 150, row: 1 },
    { t: "retries", x: 220, row: 0 },
    { t: "scaling", x: 290, row: 1 },
    { t: "migrations", x: 360, row: 0 },
    { t: "API docs", x: 430, row: 1 },
    { t: "isolation", x: 500, row: 0 },
  ];
  const car = (cls: string) => (
    <g className={`s-car ${cls}`}>
      <rect x="-18" y="-9" width="36" height="18" rx="6" className="s-car-body" />
      <rect x="2" y="-7" width="9" height="14" rx="2.5" className="s-car-glass" />
      <rect x="-14" y="-6" width="6" height="12" rx="2" className="s-car-glass" />
    </g>
  );
  return (
    <svg
      viewBox="0 0 560 300"
      className="scene"
      role="img"
      aria-label="On the general-purpose road, a car weaves around CPU and memory alerts, database load, tests, CI and deploys, slow responses, and a huge bill. On the Aleph road, a car drives straight past auth, transactions, retries, scaling, migrations, API docs, and isolation, all built in."
    >
      <text x="16" y="20" className="s-label">
        general-purpose
      </text>
      <rect x="16" y="30" width="528" height="104" rx="6" className="s-road" />
      <line x1="16" y1="82" x2="544" y2="82" className="s-lane" />
      <line x1="530" y1="30" x2="530" y2="134" className="s-finish" />
      {obstacles.map((o) => {
        const w = Math.max(...o.t.map((line) => line.length)) * 6.6 + 14;
        const h = o.t.length * 11 + 7;
        // Hug the road's edge, clear of the middle lane the car crosses between obstacles.
        const y = o.up ? 33 : 131 - h;
        return (
          <g key={o.t.join(" ")}>
            <rect x={o.x - w / 2} y={y} width={w} height={h} rx="3" className="s-obstacle" />
            <text y={y + 12} className="s-tiny s-warn-text" textAnchor="middle">
              {o.t.map((line, i) => (
                <tspan key={line} x={o.x} dy={i === 0 ? 0 : 11}>
                  {line}
                </tspan>
              ))}
            </text>
          </g>
        );
      })}
      {car("s-car-a")}

      <text x="16" y="170" className="s-label">
        aleph
      </text>
      <rect x="16" y="180" width="528" height="56" rx="6" className="s-road" />
      <line x1="16" y1="208" x2="544" y2="208" className="s-lane" />
      <line x1="530" y1="180" x2="530" y2="236" className="s-finish" />
      {features.map((f) => {
        const w = f.t.length * 6.6 + 24;
        const y = f.row === 0 ? 246 : 272;
        const delay = ((f.x - 24) / 506) * 3.6;
        return (
          <g key={f.t}>
            <rect
              x={f.x - w / 2}
              y={y}
              width={w}
              height="20"
              rx="10"
              className="s-feature"
              style={{ animationDelay: `${delay}s` }}
            />
            <text x={f.x} y={y + 14} className="s-tiny" textAnchor="middle">
              ✔ {f.t}
            </text>
          </g>
        );
      })}
      {car("s-car-b")}
    </svg>
  );
}

/** app.ts compiles to a manifest, which a shared platform runs next to other apps. */
export function HowScene() {
  const tiles = Array.from({ length: 9 }, (_, i) => i);
  return (
    <svg
      viewBox="0 18 560 228"
      className="scene"
      role="img"
      aria-label="You write app.ts; Aleph compiles it to manifest.json; a platform runs it beside other apps on one core and one database. OpenAPI, a typed client, and MCP tools are derived from the manifest too."
    >
      <text x="64" y="88" className="s-tiny s-muted" textAnchor="middle">
        you write
      </text>
      <rect x="16" y="100" width="96" height="44" rx="6" className="s-card s-strong-card" />
      <text x="64" y="126" className="s-file" textAnchor="middle">
        app.ts
      </text>
      <path d="M112 122 L150 122" className="s-edge" />
      <path d="M112 122 L150 122" pathLength={1} className="s-flow" />

      <text x="215" y="88" className="s-tiny s-muted" textAnchor="middle">
        aleph builds
      </text>
      <rect x="150" y="100" width="130" height="44" rx="6" className="s-card" />
      <text x="215" y="119" className="s-file" textAnchor="middle">
        manifest.json
      </text>
      <text x="215" y="135" className="s-tiny s-muted" textAnchor="middle">
        data, not code
      </text>

      <path d="M280 122 L316 122" className="s-edge" />
      <path
        d="M280 122 L316 122"
        pathLength={1}
        className="s-flow"
        style={{ animationDelay: "0.5s" }}
      />
      <path d="M215 144 L215 194" className="s-edge" />
      <path
        d="M215 144 L215 194"
        pathLength={1}
        className="s-flow"
        style={{ animationDelay: "0.8s" }}
      />
      <text x="215" y="212" className="s-tiny s-muted" textAnchor="middle">
        also derived:
      </text>
      <text x="215" y="230" className="s-tiny" textAnchor="middle">
        OpenAPI · typed client · MCP
      </text>

      <text x="316" y="30" className="s-label">
        platform
      </text>
      <rect x="316" y="40" width="228" height="176" rx="6" className="s-card" />
      {tiles.map((i) => {
        const x = 330 + (i % 3) * 70;
        const y = 54 + Math.floor(i / 3) * 38;
        const mine = i === 4;
        return (
          <g key={i}>
            <rect
              x={x}
              y={y}
              width="62"
              height="30"
              rx="4"
              className={mine ? "s-app s-mine" : "s-app"}
            />
            <text
              x={x + 31}
              y={y + 19}
              className={mine ? "s-tiny" : "s-tiny s-muted"}
              textAnchor="middle"
            >
              {mine ? "your app" : "app"}
            </text>
          </g>
        );
      })}
      <line x1="330" y1="180" x2="530" y2="180" className="s-edge" />
      <text x="430" y="203" className="s-tiny s-muted" textAnchor="middle">
        one core · one database
      </text>
    </svg>
  );
}

/** A request commits as one transaction and its effects go out once; a crash leaves nothing. */
export function TransactionScene() {
  const effects = ["event", "email", "job"];
  const lane = (y: number, delay: number, rollback = false) => (
    <>
      <rect x="16" y={y - 14} width="84" height="28" rx="14" className="s-pill" />
      <text x="58" y={y + 4} className="s-tiny" textAnchor="middle">
        request
      </text>
      <path d={`M100 ${y} L130 ${y}`} className="s-edge" />
      <path
        d={`M100 ${y} L130 ${y}`}
        pathLength={1}
        className="s-flow"
        style={{ animationDelay: `${delay}s` }}
      />
      <rect x="130" y={y - 22} width="150" height="44" rx="6" className="s-card" />
      <text x="205" y={y - 4} className="s-tiny s-muted" textAnchor="middle">
        one transaction
      </text>
      {[0, 1, 2].map((k) => (
        <rect
          key={k}
          x={176 + k * 22}
          y={y + 4}
          width="14"
          height="8"
          rx="2"
          className={rollback ? "s-write s-rollback" : "s-write"}
          style={rollback ? undefined : { animationDelay: `${delay + 0.3 + k * 0.15}s` }}
        />
      ))}
      <path d={`M280 ${y} L310 ${y}`} className="s-edge" />
      <path
        d={`M280 ${y} L310 ${y}`}
        pathLength={1}
        className="s-flow"
        style={{ animationDelay: `${delay + 0.6}s` }}
      />
    </>
  );
  return (
    <svg
      viewBox="0 0 560 230"
      className="scene"
      role="img"
      aria-label="A request runs as one transaction; after it commits, its event, email, and job each go out exactly once. A request that crashes is rolled back and sends nothing."
    >
      {lane(64, 0)}
      <rect x="310" y="50" width="80" height="28" rx="14" className="s-pill s-ok-pill" />
      <text x="350" y="68" className="s-tiny s-ok-text" textAnchor="middle">
        ✔ commit
      </text>
      {effects.map((e, i) => {
        const y = 22 + i * 30;
        const d = `M390 64 C 414 64, 414 ${y + 11}, 438 ${y + 11}`;
        return (
          <g key={e}>
            <path d={d} className="s-edge" />
            <path
              d={d}
              pathLength={1}
              className="s-flow"
              style={{ animationDelay: `${1.1 + i * 0.12}s` }}
            />
            <rect
              x="438"
              y={y}
              width="106"
              height="22"
              rx="4"
              className="s-card s-out"
              style={{ animationDelay: `${1.1 + i * 0.12}s` }}
            />
            <text x="450" y={y + 15} className="s-tiny">
              {e}
            </text>
            <text x="532" y={y + 15} className="s-tiny s-muted" textAnchor="end">
              ×1
            </text>
          </g>
        );
      })}

      {lane(166, 1, true)}
      <g className="s-crash">
        <rect x="310" y="152" width="80" height="28" rx="14" className="s-pill s-err-pill" />
        <text x="350" y="170" className="s-tiny s-err-text" textAnchor="middle">
          ✕ crash
        </text>
        <text x="410" y="162" className="s-tiny s-err-text">
          rolled back,
        </text>
        <text x="410" y="178" className="s-tiny s-err-text">
          nothing sent
        </text>
      </g>
      <text x="16" y="222" className="s-tiny s-muted">
        no half-written state, no retry logic to write
      </text>
    </svg>
  );
}

/** One manifest, any runtime: the TypeScript is only how you write it. */
export function RuntimeScene() {
  const runtimes = [
    { y: 46, name: "aleph-core", note: "Rust, native: the platform", strong: true },
    { y: 112, name: "aleph-core", note: "Rust as WASM: CLI, aleph dev, tests", strong: false },
    { y: 178, name: "your engine", note: "Go, Zig, anything that reads the spec", strong: false },
  ];
  return (
    <svg
      viewBox="0 18 560 228"
      className="scene"
      role="img"
      aria-label="You write app.ts in TypeScript; Aleph compiles it to manifest.json; the same manifest runs on the Rust core natively, on the Rust core as WASM, or on any other engine that implements the open spec."
    >
      <text x="64" y="104" className="s-tiny s-muted" textAnchor="middle">
        you write
      </text>
      <rect x="16" y="116" width="96" height="44" rx="6" className="s-card s-strong-card" />
      <text x="64" y="142" className="s-file" textAnchor="middle">
        app.ts
      </text>
      <path d="M112 138 L146 138" className="s-edge" />
      <path d="M112 138 L146 138" pathLength={1} className="s-flow" />

      <text x="211" y="104" className="s-tiny s-muted" textAnchor="middle">
        aleph builds
      </text>
      <rect x="146" y="116" width="130" height="44" rx="6" className="s-card" />
      <text x="211" y="135" className="s-file" textAnchor="middle">
        manifest.json
      </text>
      <text x="211" y="151" className="s-tiny s-muted" textAnchor="middle">
        no runtime inside
      </text>

      {runtimes.map((r, i) => (
        <g key={r.note}>
          <path d={`M276 138 C 300 138, 300 ${r.y + 22}, 324 ${r.y + 22}`} className="s-edge" />
          <path
            d={`M276 138 C 300 138, 300 ${r.y + 22}, 324 ${r.y + 22}`}
            pathLength={1}
            className="s-flow"
            style={{ animationDelay: `${0.5 + i * 0.3}s` }}
          />
          <rect
            x="324"
            y={r.y}
            width="220"
            height="44"
            rx="6"
            className={r.strong ? "s-card s-strong-card" : "s-card"}
          />
          <text x="434" y={r.y + 19} className="s-file" textAnchor="middle">
            {r.name}
          </text>
          <text x="434" y={r.y + 35} className="s-tiny s-muted" textAnchor="middle">
            {r.note}
          </text>
        </g>
      ))}
    </svg>
  );
}
