/**
 * The homepage's diagrams: small SVG scenes animated with CSS (see globals.css, "Scenes"). Each
 * shows one consequence of Aleph's design. Motion stops for people who prefer reduced motion.
 */

/** Declarative: three lines of code become a plan, and the plan has a fixed ceiling. */
export function DescribeScene() {
  return (
    <svg
      viewBox="0 0 520 260"
      className="scene"
      role="img"
      aria-label="Code becomes a plan with a known maximum cost"
    >
      <rect x="12" y="40" width="200" height="140" rx="10" className="s-card" />
      <text x="28" y="70" className="s-code">
        a.route("GET /todos")
      </text>
      <text x="28" y="94" className="s-code">
        {"  .handle(($) =>"}
      </text>
      <text x="28" y="118" className="s-code">
        {"    Todo.query({"}
      </text>
      <text x="28" y="142" className="s-code">
        {"      prefix: [$.caller],"}
      </text>
      <text x="28" y="166" className="s-code s-hl">
        {"      limit: 50 }))"}
      </text>
      <path d="M212 110 C 250 110, 250 110, 288 110" className="s-line s-draw" />
      <g className="s-plan">
        <circle cx="320" cy="70" r="14" className="s-node" />
        <circle cx="300" cy="140" r="14" className="s-node s-d1" />
        <circle cx="350" cy="140" r="14" className="s-node s-d2" />
        <path d="M320 84 L302 126 M320 84 L348 126" className="s-edge" />
        <text x="320" y="74" className="s-tiny" textAnchor="middle">
          plan
        </text>
      </g>
      <text x="400" y="52" className="s-label">
        worst case
      </text>
      <rect x="400" y="62" width="100" height="150" rx="6" className="s-gauge" />
      <line x1="394" y1="112" x2="506" y2="112" className="s-cap" />
      <text x="400" y="104" className="s-tiny s-strong">
        ≤ 50 reads
      </text>
      <rect x="412" y="112" width="76" height="100" className="s-level" />
    </svg>
  );
}

/** Predictable: requests stay under their ceiling; a plan that could exceed it never ships. */
export function PredictableScene() {
  const bars = [62, 40, 78, 30, 55, 70, 46, 66, 35, 58];
  return (
    <svg
      viewBox="0 0 520 260"
      className="scene"
      role="img"
      aria-label="Every request stays under its limit; an over-budget plan is rejected before deploy"
    >
      <line x1="20" y1="70" x2="500" y2="70" className="s-cap" />
      <text x="20" y="60" className="s-tiny s-strong">
        the route's bound
      </text>
      {bars.map((h, i) => (
        <rect
          // biome-ignore lint/suspicious/noArrayIndexKey: static bars
          key={i}
          x={30 + i * 34}
          y={220 - h * 1.8}
          width="18"
          height={h * 1.8}
          rx="3"
          className="s-bar"
          style={{ animationDelay: `${i * 0.35}s` }}
        />
      ))}
      <g className="s-reject">
        <rect x="380" y="30" width="18" height="190" rx="3" className="s-bar-over" />
        <rect x="408" y="96" width="104" height="54" rx="8" className="s-card" />
        <text x="420" y="118" className="s-tiny s-err">
          ✖ over_budget
        </text>
        <text x="420" y="138" className="s-tiny">
          caught by aleph check
        </text>
      </g>
      <line x1="20" y1="220" x2="500" y2="220" className="s-axis" />
    </svg>
  );
}

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
            server · db · cache
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

/** Lower cost: paying for work that happens instead of capacity that waits. */
export function CostScene() {
  return (
    <svg
      viewBox="0 0 520 260"
      className="scene"
      role="img"
      aria-label="Cost follows actual usage instead of provisioned capacity"
    >
      <line x1="40" y1="220" x2="500" y2="220" className="s-axis" />
      <line x1="40" y1="40" x2="40" y2="220" className="s-axis" />
      <text x="48" y="58" className="s-tiny s-muted">
        provisioned: paid whether used or not
      </text>
      <line x1="40" y1="70" x2="500" y2="70" className="s-provisioned" />
      <path
        d="M40 210 C 80 205, 100 196, 130 200 S 190 170, 220 186 S 280 150, 310 172 S 380 120, 410 150 S 470 132, 500 140"
        className="s-usage s-draw-long"
      />
      <text x="330" y="112" className="s-tiny s-strong">
        shared: pay for what runs
      </text>
      <text x="44" y="240" className="s-tiny s-muted">
        time
      </text>
    </svg>
  );
}

/** Agent-friendly: an agent writes, the checker answers precisely, the agent fixes it. */
export function AgentScene() {
  return (
    <svg
      viewBox="0 0 520 260"
      className="scene"
      role="img"
      aria-label="An agent writes a route, gets a precise diagnostic, and fixes it"
    >
      <rect x="20" y="24" width="480" height="196" rx="10" className="s-term" />
      <circle cx="40" cy="42" r="5" className="s-dot" />
      <circle cx="56" cy="42" r="5" className="s-dot" />
      <circle cx="72" cy="42" r="5" className="s-dot" />
      <g className="s-step s-step1">
        <text x="40" y="78" className="s-code s-on-term">
          agent ▸ Todo.query({"{"} prefix: [$.caller], limit: 5000 {"}"})
        </text>
      </g>
      <g className="s-step s-step2">
        <text x="40" y="108" className="s-code s-on-term s-err">
          ✖ over_budget: up to 5000 reads, over the limit of 250
        </text>
        <text x="40" y="128" className="s-code s-on-term s-muted-term">
          {"  "}Lower the bounds that drive it (query limits) or split the work.
        </text>
      </g>
      <g className="s-step s-step3">
        <text x="40" y="160" className="s-code s-on-term">
          agent ▸ Todo.page({"{"} prefix: [$.caller], limit: 50 {"}"})
        </text>
      </g>
      <g className="s-step s-step4">
        <text x="40" y="190" className="s-code s-on-term s-ok">
          ✔ valid · get_todos: 50 reads (20%)
        </text>
      </g>
    </svg>
  );
}

/** One description, everything generated from it. */
export function GeneratedScene() {
  const outputs = ["OpenAPI", "typed client", "MCP tools", "examples as tests", "schema checks"];
  return (
    <svg
      viewBox="0 0 520 260"
      className="scene"
      role="img"
      aria-label="One declaration generates OpenAPI, a typed client, MCP tools, tests, and schema checks"
    >
      <rect x="20" y="104" width="130" height="56" rx="10" className="s-card s-strong-card" />
      <text x="85" y="137" className="s-label" textAnchor="middle">
        app.ts
      </text>
      {outputs.map((name, i) => {
        const y = 36 + i * 46;
        return (
          <g key={name}>
            <path d={`M150 132 C 220 132, 230 ${y + 16}, 300 ${y + 16}`} className="s-edge" />
            <path
              d={`M150 132 C 220 132, 230 ${y + 16}, 300 ${y + 16}`}
              className="s-flow"
              style={{ animationDelay: `${i * 0.5}s` }}
            />
            <rect
              x="300"
              y={y}
              width="190"
              height="32"
              rx="8"
              className="s-card s-out"
              style={{ animationDelay: `${i * 0.5}s` }}
            />
            <text x="314" y={y + 21} className="s-tiny">
              {name}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
