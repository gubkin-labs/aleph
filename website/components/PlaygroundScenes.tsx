import type { CSSProperties, ReactNode } from "react";
import { Visible } from "@/components/Visible";

/**
 * The animations in the homepage playground. Every scene is built from the same pieces on the
 * same stage: callers on the left, tall cards for the things they reach (a gate, a server, a
 * database, a mailbox), and a footer line that names each one. A wide pill is a big request, red
 * is what goes wrong and green is what is accepted. A scene plays once (the playground remounts
 * it to replay) and ends on a still that says the same thing, which is also what people who
 * prefer reduced motion see. Scenes that compare two worlds play the typical one first, fade it
 * out, then play Aleph's. Times are seconds from the start; see globals.css, "Playground".
 */

const vars = (o: Record<string, string | number>) => o as unknown as CSSProperties;
type Tone = "ok" | "err" | "plain";

// The stage: every card is as tall as the stage; marks and labels sit on fixed lines.
const TOP = 14;
const H = 120;
const ONE = TOP + H / 2;
const L1 = 40;
const L2 = 108;
const MARK_Y = 112;
const LABEL_Y = 160;

type Stage = { w: number; users: number; mid: number; db: number };
const WIDE: Stage = { w: 1000, users: 60, mid: 500, db: 940 };

/** How far a request starting at `from` with width `w` travels to stop at `edge`. */
const reach = (edge: number, from: number, w: number) => edge - 4 - (from + w);

function Scene({
  aria,
  w = 560,
  h = 172,
  wide,
  big,
  className,
  children,
}: {
  aria: string;
  w?: number;
  h?: number;
  wide?: boolean;
  className?: string;
  /** A scene that is the whole tab: it needs more room to stay readable on narrow screens. */
  big?: boolean;
  children: ReactNode;
}) {
  return (
    <figure
      className={["demo", "pg-scene", wide && "pg-scene-wide", big && "pg-scene-big", className]
        .filter(Boolean)
        .join(" ")}
    >
      <Visible threshold={big ? 0.1 : 0.6}>
        <svg viewBox={`0 0 ${w} ${h}`} className="scene" role="img" aria-label={aria}>
          {children}
        </svg>
      </Visible>
    </figure>
  );
}

// ─── Shapes ─────────────────────────────────────────────────────────────────────

function Label({
  x,
  y = LABEL_Y,
  t,
  children,
}: {
  x: number;
  y?: number;
  t?: number;
  children: ReactNode;
}) {
  return (
    <text
      x={x}
      y={y}
      textAnchor="middle"
      className={t === undefined ? "pg-label" : "pg-label pg-at"}
      style={t === undefined ? undefined : vars({ "--t": `${t}s` })}
    >
      {children}
    </text>
  );
}

/** A small caption that names a phase of the scene, in the top-left corner clear of the cards. */
function Tag({ x = 30, t, children }: { x?: number; t?: number; children: ReactNode }) {
  return (
    <text
      x={x}
      y={11}
      textAnchor="start"
      className={t === undefined ? "pg-tag" : "pg-tag pg-at"}
      style={t === undefined ? undefined : vars({ "--t": `${t}s` })}
    >
      {children}
    </text>
  );
}

/** Everything inside fades out at `t`; it is gone from the final still. */
function Out({ t, children }: { t: number; children: ReactNode }) {
  return (
    <g className="pg-out" style={vars({ "--t": `${t}s` })}>
      {children}
    </g>
  );
}

/** A caller: a solid circle when signed in, a dashed one when anonymous. */
function User({ cx, y, anon }: { cx: number; y: number; anon?: boolean }) {
  const cls = anon ? "pg-shape pg-anon" : "pg-shape";
  return (
    <g>
      <circle cx={cx} cy={y} r={14} className={cls} />
      <circle cx={cx} cy={y - 3} r={4} className={anon ? "pg-anon-fill" : "pg-shape-fill"} />
      <path d={`M${cx - 7} ${y + 8} a7 5 0 0 1 14 0`} className={cls} />
    </g>
  );
}

/** A piece of code: a page with lines. */
function Doc({ cx, y, s = 1 }: { cx: number; y: number; s?: number }) {
  const w = 38 * s;
  const h = 48 * s;
  return (
    <g>
      <rect x={cx - w / 2} y={y - h / 2} width={w} height={h} rx={5 * s} className="s-card" />
      {[0.25, 0.44, 0.63, 0.82].map((f, i) => (
        <line
          key={f}
          x1={cx - w / 2 + 8 * s}
          y1={y - h / 2 + h * f}
          x2={cx + w / 2 - 8 * s - (i % 2) * 8 * s}
          y2={y - h / 2 + h * f}
          className="s-edge"
        />
      ))}
    </g>
  );
}

function Card({ cx, w = 56, strong }: { cx: number; w?: number; strong?: boolean }) {
  return (
    <rect
      x={cx - w / 2}
      y={TOP}
      width={w}
      height={H}
      rx={8}
      className={strong ? "s-card s-strong-card" : "s-card"}
    />
  );
}

/** A glyph centered in a card: a short word or symbol. */
function Glyph({ cx, children, size = 17 }: { cx: number; children: ReactNode; size?: number }) {
  return (
    <text x={cx} y={ONE + size * 0.35} textAnchor="middle" className="pg-glyph" fontSize={size}>
      {children}
    </text>
  );
}

function Keyhole({ cx, cy = ONE }: { cx: number; cy?: number }) {
  return (
    <g>
      <circle cx={cx} cy={cy - 4} r={6} className="pg-shape" />
      <line x1={cx} y1={cy + 2} x2={cx} y2={cy + 14} className="pg-shape" />
    </g>
  );
}

/** Your server: a card with two bars. */
function Server({ cx }: { cx: number }) {
  return (
    <g>
      <Card cx={cx} />
      <line x1={cx - 16} y1={TOP + 14} x2={cx + 16} y2={TOP + 14} className="s-edge" />
      <line x1={cx - 16} y1={TOP + 25} x2={cx + 16} y2={TOP + 25} className="s-edge" />
    </g>
  );
}

/** A database: a cylinder. */
function Db({ cx, y = TOP, h = H, w = 64 }: { cx: number; y?: number; h?: number; w?: number }) {
  const r = Math.min(9, h / 4);
  const x = cx - w / 2;
  return (
    <g>
      <path
        d={`M${x} ${y + r} v${h - 2 * r} a${w / 2} ${r} 0 0 0 ${w} 0 v${-(h - 2 * r)}`}
        className="s-card"
      />
      <ellipse cx={cx} cy={y + r} rx={w / 2} ry={r} className="s-card" />
    </g>
  );
}

function Envelope({ cx, cy, w = 36 }: { cx: number; cy: number; w?: number }) {
  const h = w * 0.66;
  return (
    <g>
      <rect x={cx - w / 2} y={cy - h / 2} width={w} height={h} rx={3} className="pg-env" />
      <polyline
        points={`${cx - w / 2 + 2},${cy - h / 2 + 3} ${cx},${cy + 2} ${cx + w / 2 - 2},${cy - h / 2 + 3}`}
        className="pg-line"
      />
    </g>
  );
}

/** A cog; the teeth are a dashed ring. */
function Gear({ cx, cy, r = 11 }: { cx: number; cy: number; r?: number }) {
  return (
    <g>
      <circle
        cx={cx}
        cy={cy}
        r={r + 3}
        className="pg-teeth"
        strokeDasharray={`${(2 * Math.PI * (r + 3)) / 16} ${(2 * Math.PI * (r + 3)) / 16}`}
      />
      <circle cx={cx} cy={cy} r={r} className="pg-shape" />
      <circle cx={cx} cy={cy} r={r / 3} className="pg-shape" />
    </g>
  );
}

/** A box with a lid: a compiled program. */
function Cube({ cx, cy, s = 15 }: { cx: number; cy: number; s?: number }) {
  const p = (x: number, y: number) => `${cx + x * s},${cy + y * s}`;
  return (
    <g className="pg-shape">
      <polygon points={`${p(0, -1)} ${p(0.9, -0.5)} ${p(0, 0)} ${p(-0.9, -0.5)}`} />
      <polygon points={`${p(-0.9, -0.5)} ${p(0, 0)} ${p(0, 1)} ${p(-0.9, 0.5)}`} />
      <polygon points={`${p(0.9, -0.5)} ${p(0, 0)} ${p(0, 1)} ${p(0.9, 0.5)}`} />
    </g>
  );
}

/** A coin with a dollar sign. */
function Coin({ cx, cy, t, r = 9 }: { cx: number; cy: number; t: number; r?: number }) {
  return (
    <g className="pg-at" style={vars({ "--t": `${t}s` })}>
      <circle cx={cx} cy={cy} r={r} className="pg-coin" />
      <text
        x={cx}
        y={cy + r * 0.36}
        textAnchor="middle"
        className="pg-coin-text"
        fontSize={r * 1.15}
      >
        $
      </text>
    </g>
  );
}

/** A card's outline, or a lane of it, flashing a verdict color. */
function Flash({
  x,
  y,
  w,
  h,
  tone,
  t,
}: {
  x: number;
  y: number;
  w: number;
  h: number;
  tone: Tone;
  t: number;
}) {
  return (
    <rect
      x={x - 2}
      y={y - 2}
      width={w + 4}
      height={h + 4}
      rx={9}
      className={`pg-flash pg-flash-${tone} pg-at`}
      style={vars({ "--t": `${t}s` })}
    />
  );
}

const flashCard = (cx: number, t: number, tone: Tone, w = 56) => (
  <Flash x={cx - w / 2} y={TOP} w={w} h={H} tone={tone} t={t} />
);

/** A verdict: a red cross or a green check. */
function Mark({
  cx,
  cy = MARK_Y,
  ok,
  t,
  k = 1,
}: {
  cx: number;
  cy?: number;
  ok?: boolean;
  t: number;
  k?: number;
}) {
  const a = 4.5 * k;
  return (
    <g className="pg-at" style={vars({ "--t": `${t}s` })}>
      <circle
        cx={cx}
        cy={cy}
        r={12 * k}
        className={ok ? "pg-mark-bg pg-mark-bg-ok" : "pg-mark-bg pg-mark-bg-err"}
      />
      {ok ? (
        <polyline
          points={`${cx - 5 * k},${cy} ${cx - 1.5 * k},${cy + 4 * k} ${cx + 5.5 * k},${cy - 4 * k}`}
          className="pg-mark pg-mark-ok"
        />
      ) : (
        <>
          <line x1={cx - a} y1={cy - a} x2={cx + a} y2={cy + a} className="pg-mark pg-mark-err" />
          <line x1={cx + a} y1={cy - a} x2={cx - a} y2={cy + a} className="pg-mark pg-mark-err" />
        </>
      )}
    </g>
  );
}

/** A small page of code moving along the lane: a version of the app on its way to shipping. */
function DocPkt({
  from,
  tone = "plain",
  t,
  dx,
  dur = 1,
  o = 0,
}: {
  from: number;
  tone?: Tone;
  t: number;
  dx: number;
  dur?: number;
  o?: number;
}) {
  return (
    <g transform={`translate(${from} ${ONE - 15})`}>
      <g
        className="pg-fly"
        style={vars({
          "--t": `${t}s`,
          "--dx": `${dx}px`,
          "--dy": "0px",
          "--dur": `${dur}s`,
          "--o": o,
        })}
      >
        <rect width={22} height={30} rx={3} className={`pg-pkt pg-pkt-${tone}`} />
        <line x1={5} y1={9} x2={17} y2={9} className="pg-docline" />
        <line x1={5} y1={15} x2={17} y2={15} className="pg-docline" />
        <line x1={5} y1={21} x2={12} y2={21} className="pg-docline" />
      </g>
    </g>
  );
}

/**
 * The Aleph half of the scenes that share one path for both code panes: the code itself is the
 * traveller. A version that breaks a limit is turned away at the check, before it ships; the
 * fixed version passes the check and reaches the server. Times line up with the Aleph code's
 * verdicts (rejected at about 7.5s, accepted at about 11s).
 */
function Ship({ st, gate, serverMark = true }: { st: Stage; gate: number; serverMark?: boolean }) {
  const from = st.users + 24;
  return (
    <g>
      <g className="pg-at" style={vars({ "--t": "5.2s" })}>
        <Doc cx={st.users} y={ONE} />
        <Card cx={gate} strong />
        <Keyhole cx={gate} />
      </g>
      <DocPkt from={from} tone="err" t={6.3} dx={reach(gate - 28, from, 22)} dur={1} />
      {flashCard(gate, 7.4, "err")}
      <Mark cx={gate} t={7.4} />
      <DocPkt from={from} tone="ok" t={9.4} dx={reach(st.mid - 28, from, 22)} dur={1.4} />
      <Mark cx={gate} cy={L1} ok t={10.1} />
      {serverMark && <Mark cx={st.mid} ok t={10.9} />}
      <Label x={st.users} t={5.2}>
        your code
      </Label>
      <Label x={gate} t={5.2}>
        aleph check
      </Label>
    </g>
  );
}

/** A row stored in a database. */
function Row({ cx, y, t, tone = "plain" }: { cx: number; y: number; t: number; tone?: Tone }) {
  return (
    <rect
      x={cx - 22}
      y={y}
      width={44}
      height={10}
      rx={3}
      className={`pg-pkt pg-pkt-${tone} pg-at`}
      style={vars({ "--t": `${t}s` })}
    />
  );
}

/** A request moving along a lane; its width is how much it asks for. `o` is how visible it is on arrival. */
function Pkt({
  from,
  lane = ONE,
  w,
  tone = "plain",
  t,
  dx,
  dy = 0,
  dur = 1,
  o = 0,
}: {
  from: number;
  lane?: number;
  w: number;
  tone?: Tone;
  t: number;
  dx: number;
  dy?: number;
  dur?: number;
  o?: number;
}) {
  return (
    <g transform={`translate(${from} ${lane - 7})`}>
      <g
        className="pg-fly"
        style={vars({
          "--t": `${t}s`,
          "--dx": `${dx}px`,
          "--dy": `${dy}px`,
          "--dur": `${dur}s`,
          "--o": o,
        })}
      >
        <rect width={w} height={14} rx={7} className={`pg-pkt pg-pkt-${tone}`} />
      </g>
    </g>
  );
}

/** A small dot, like a query, that flies and is absorbed. */
function Dot({
  x,
  y,
  t,
  dx,
  dy = 0,
  dur = 0.7,
  tone = "plain",
  o = 0,
  loop,
}: {
  x: number;
  y: number;
  t: number;
  dx: number;
  dy?: number;
  dur?: number;
  tone?: Tone;
  o?: number;
  /** Keep flying for as long as the scene is on screen. */
  loop?: boolean;
}) {
  return (
    <g transform={`translate(${x} ${y})`}>
      <g
        className={loop ? "pg-flyloop" : "pg-fly"}
        style={vars({
          "--t": `${t}s`,
          "--dx": `${dx}px`,
          "--dy": `${dy}px`,
          "--dur": `${dur}s`,
          "--o": o,
        })}
      >
        <circle r={4.5} className={`pg-dot pg-dot-${tone}`} />
      </g>
    </g>
  );
}

/** A meter that fills up to `to` (0 to 1) from `t`. `hot` makes it red. */
function Meter({
  x,
  y,
  h = 64,
  t,
  d,
  to,
  hot,
}: {
  x: number;
  y: number;
  h?: number;
  t: number;
  d: number;
  to: number;
  hot?: boolean;
}) {
  return (
    <g>
      <rect x={x} y={y} width={10} height={h} rx={3} className="s-track" />
      <rect
        x={x}
        y={y}
        width={10}
        height={h}
        rx={3}
        className={hot ? "pg-load pg-rise" : "pg-calm pg-rise"}
        style={vars({ "--t": `${t}s`, "--d": `${d}s`, "--to": to })}
      />
    </g>
  );
}

// ─── 1. Infra protection ────────────────────────────────────────────────────────

const GATE = 270;

/**
 * One request path for both code panes. First what a typical backend does with a request for far
 * too much: it accepts it and floods the database. Then Aleph: the code that would allow that is
 * rejected before it ships, and the bounded version goes through.
 */
export function InfraScene() {
  const st = WIDE;
  const midL = st.mid - 28;
  const dbL = st.db - 32;
  const from = st.users + 24;
  const dots = [0, 1, 2, 3, 4, 5, 6, 7, 8];
  return (
    <Scene
      wide
      w={st.w}
      aria="Users send a request to a server, which queries a database. A typical backend accepts a request for far too much and floods the database until it fails. With Aleph, the code that would allow that is rejected before it ships, and the bounded version passes the check and reaches the server."
    >
      <Server cx={st.mid} />
      <Db cx={st.db} />

      <Out t={4.9}>
        <Tag>a typical backend</Tag>
        <User cx={st.users} y={ONE} />
        <Label x={st.users}>users</Label>
        <rect x={st.db - 20} y={TOP + 26} width={40} height={7} rx={3} className="s-track" />
        <rect
          x={st.db - 20}
          y={TOP + 26}
          width={40}
          height={7}
          rx={3}
          className="pg-load pg-grow"
          style={vars({ "--t": "1.7s", "--d": "2.6s" })}
        />
        <Pkt from={from} w={64} t={0.4} dx={reach(midL, from, 64)} dur={0.8} />
        <Mark cx={st.mid} ok t={1.3} />
        {dots.map((i) => (
          <Dot
            key={i}
            x={st.mid + 32}
            y={ONE}
            t={1.6 + i * 0.25}
            dx={dbL - 4 - (st.mid + 32)}
            dy={-16 + i * 4}
          />
        ))}
        <Mark cx={st.db} t={4.2} />
      </Out>

      <Tag t={5.2}>with aleph</Tag>
      <Ship st={st} gate={GATE} />
      {[0, 1, 2].map((i) => (
        <Dot
          key={i}
          x={st.mid + 32}
          y={ONE}
          t={11 + i * 0.12}
          dx={dbL - 4 - (st.mid + 32)}
          dy={-4 + i * 4}
          dur={0.6}
          tone="ok"
        />
      ))}
      <rect
        x={st.db - 20}
        y={TOP + 26}
        width={40}
        height={7}
        rx={3}
        className="s-track pg-at"
        style={vars({ "--t": "11.5s" })}
      />
      <rect
        x={st.db - 20}
        y={TOP + 26}
        width={9}
        height={7}
        rx={3}
        className="pg-calm pg-at"
        style={vars({ "--t": "11.6s" })}
      />
      <Row cx={st.db} y={60} t={11.7} tone="ok" />
      <Row cx={st.db} y={72} t={11.8} tone="ok" />
      <Row cx={st.db} y={84} t={11.9} tone="ok" />
      <Mark cx={st.db} ok t={12} />

      <Label x={st.mid}>server</Label>
      <Label x={st.db}>db</Label>
    </Scene>
  );
}

// ─── 2. Builtin guardrails ──────────────────────────────────────────────────────

/** A server's two meters: cpu on the left, memory on the right. */
const meters = (cx: number, hot: boolean, t: number, cpu: number, mem: number) => (
  <g>
    <Meter x={cx - 18} y={TOP + 26} t={t} d={cpu > 0.5 ? 1.6 : 0.6} to={cpu} hot={hot} />
    <Meter x={cx + 8} y={TOP + 26} t={t + 0.2} d={mem > 0.5 ? 2.6 : 0.6} to={mem} hot={hot} />
    <text x={cx - 13} y={TOP + H - 10} textAnchor="middle" className="pg-tiny">
      cpu
    </text>
    <text x={cx + 13} y={TOP + H - 10} textAnchor="middle" className="pg-tiny">
      mem
    </text>
  </g>
);

/**
 * One server for both code panes. A request that makes the server loop over a huge list pegs its
 * cpu and memory while everyone else queues. With Aleph the code that allows that is rejected
 * before it ships, so the server's meters stay low.
 */
export function GuardrailsScene() {
  const st = WIDE;
  const midL = st.mid - 28;
  const from = st.users + 24;
  return (
    <Scene
      wide
      w={st.w}
      aria="A request makes a server loop over a huge list. Without guardrails the server's cpu and memory spike to the limit and other requests queue behind it. With Aleph, the code that allows that is rejected before it ships, and the server's meters stay low."
    >
      <Server cx={st.mid} />
      <Db cx={st.db} />

      <Out t={4.9}>
        <Tag>a typical backend</Tag>
        <User cx={st.users} y={ONE} />
        <Label x={st.users}>users</Label>
        {meters(st.mid, true, 1.3, 1, 0.92)}
        <Pkt from={from} w={64} t={0.4} dx={reach(midL, from, 64)} dur={0.8} />
        {flashCard(st.mid, 3, "err")}
        {[-20, 0, 20].map((dy, i) => (
          <rect
            key={dy}
            x={midL - 40}
            y={ONE + dy - 7}
            width={26}
            height={14}
            rx={7}
            className="pg-pkt pg-pkt-plain pg-at"
            style={vars({ "--t": `${3.2 + i * 0.3}s` })}
          />
        ))}
      </Out>

      <Tag t={5.2}>with aleph</Tag>
      <g className="pg-at" style={vars({ "--t": "5.2s" })}>
        {meters(st.mid, false, 10.9, 0.16, 0.2)}
      </g>
      <Ship st={st} gate={GATE} serverMark={false} />

      <Label x={st.mid}>server</Label>
      <Label x={st.db}>db</Label>
    </Scene>
  );
}

// ─── 3. Exactly once ────────────────────────────────────────────────────────────

const TXL = 190;
const TXR = 500;
const DB_X = 250;
const MAIL_X = 440;

/** A "db insert" item: a small cylinder. */
function Insert({ cx, y }: { cx: number; y: number }) {
  return <Db cx={cx} y={y - 15} h={30} w={34} />;
}

/**
 * Two users send requests at the same time, and each request is its own transaction: a db insert,
 * then an email. With Aleph both steps are in one transaction, so when it fails both are rolled
 * back; with typical code they are two steps, so the row is written and the email is lost.
 */
function exactlyScene(atomic: boolean) {
  const from = 64;
  const DB_MARK = DB_X + 38;
  const MAIL_MARK = MAIL_X + 38;
  /** `fails` is whether this user's transaction goes wrong. */
  const lane = (y: number, t0: number, fails: boolean) => (
    <g>
      <User cx={40} y={y} />
      {atomic ? (
        <rect x={TXL} y={y - 28} width={TXR - TXL} height={56} rx={8} className="pg-tx" />
      ) : (
        <>
          <rect x={TXL} y={y - 28} width={120} height={56} rx={8} className="pg-tx" />
          <rect x={TXR - 120} y={y - 28} width={120} height={56} rx={8} className="pg-tx" />
        </>
      )}
      <Pkt from={from} lane={y} w={30} t={t0} dx={reach(TXL, from, 30)} dur={0.8} />
      <g className="pg-at" style={vars({ "--t": `${t0 + 0.9}s` })}>
        <Insert cx={DB_X} y={y} />
      </g>
      <g className="pg-at" style={vars({ "--t": `${t0 + 1.2}s` })}>
        <Envelope cx={MAIL_X} cy={y} w={36} />
      </g>
      {atomic ? (
        <>
          <Flash x={TXL} y={y - 28} w={TXR - TXL} h={56} tone={fails ? "err" : "ok"} t={t0 + 2} />
          <Mark cx={DB_MARK} cy={y} ok={!fails} t={t0 + 2} k={0.85} />
          <Mark cx={MAIL_MARK} cy={y} ok={!fails} t={t0 + 2} k={0.85} />
        </>
      ) : (
        <>
          <Flash x={TXL} y={y - 28} w={120} h={56} tone="ok" t={t0 + 1.7} />
          <Mark cx={DB_MARK} cy={y} ok t={t0 + 1.7} k={0.85} />
          {fails ? (
            <>
              <Mark cx={(TXL + 120 + TXR - 120) / 2} cy={y} t={t0 + 2.4} k={0.85} />
              <Flash x={TXR - 120} y={y - 28} w={120} h={56} tone="err" t={t0 + 3} />
              <Mark cx={MAIL_MARK} cy={y} t={t0 + 3} k={0.85} />
            </>
          ) : (
            <>
              <Flash x={TXR - 120} y={y - 28} w={120} h={56} tone="ok" t={t0 + 2.6} />
              <Mark cx={MAIL_MARK} cy={y} ok t={t0 + 2.6} k={0.85} />
            </>
          )}
        </>
      )}
    </g>
  );
  return (
    <Scene
      aria={
        atomic
          ? "Two users send requests at the same time. Each request is one transaction holding a database insert and an email. One transaction succeeds, and in the other both the insert and the email are rolled back."
          : "Two users send requests at the same time. Each request is two separate transactions: a database insert, then an email. One request succeeds. In the other the insert is committed, the server fails, and the email is never sent."
      }
    >
      {lane(L1, 0.3, true)}
      {lane(L2, 0.5, false)}
      <Label x={40}>users</Label>
      <Label x={DB_X}>db insert</Label>
      <Label x={MAIL_X}>email sent</Label>
    </Scene>
  );
}

export function ExactlyAleph() {
  return exactlyScene(true);
}

export function ExactlyNode() {
  return exactlyScene(false);
}

// ─── 4. Shared infra, reduced costs ─────────────────────────────────────────────

const SH = 230;
const WW = 1000;

function MiniServer({ cx, cy }: { cx: number; cy: number }) {
  return (
    <g>
      <rect x={cx - 15} y={cy - 12} width={30} height={24} rx={4} className="s-card" />
      <line x1={cx - 9} y1={cy - 4} x2={cx + 9} y2={cy - 4} className="s-edge" />
      <line x1={cx - 9} y1={cy + 3} x2={cx + 9} y2={cy + 3} className="s-edge" />
    </g>
  );
}

function MiniCache({ cx, cy }: { cx: number; cy: number }) {
  return (
    <g>
      <rect x={cx - 13} y={cy - 12} width={26} height={24} rx={4} className="s-card" />
      <polyline
        points={`${cx + 2},${cy - 8} ${cx - 4},${cy + 1} ${cx + 1},${cy + 1} ${cx - 2},${cy + 8}`}
        className="pg-line"
      />
    </g>
  );
}

function Pop({ t, children }: { t: number; children: ReactNode }) {
  return (
    <g className="pg-at" style={vars({ "--t": `${t}s` })}>
      {children}
    </g>
  );
}

/** With Aleph: the apps share one runtime and one set of infrastructure, so the cost is shared too. */
export function SharedAleph() {
  const docs = [0, 1, 2, 3, 4, 5];
  return (
    <Scene
      wide
      w={WW}
      h={SH}
      aria="Six apps run inside one shared runtime that has a single server, database, and cache, with one dollar coin for the whole cost."
    >
      <Tag>with aleph</Tag>
      <rect x={30} y={22} width={940} height={160} rx={10} className="pg-tx" />
      {docs.map((i) => {
        const cx = 120 + i * 152;
        return (
          <Pop key={cx} t={0.3 + i * 0.25}>
            <Doc cx={cx} y={58} s={0.8} />
            <line x1={cx} y1={78} x2={cx} y2={92} className="s-edge" />
          </Pop>
        );
      })}
      <Pop t={1.9}>
        <line x1={120} y1={92} x2={880} y2={92} className="s-edge" />
        <line x1={500} y1={92} x2={500} y2={98} className="s-edge" />
        <MiniServer cx={380} cy={116} />
        <Db cx={500} y={102} h={28} w={30} />
        <MiniCache cx={620} cy={116} />
      </Pop>
      <Coin cx={500} cy={160} t={2.7} />
      <Label x={500} y={212}>
        6 apps · one runtime · shared infra
      </Label>
    </Scene>
  );
}

/** Typical platforms: every app brings its own runtime, database, and cache, and its own bill. */
export function SharedTypical() {
  const apps = [180, 500, 820];
  return (
    <Scene
      wide
      w={WW}
      h={SH}
      aria="Three apps, each in its own runtime with its own server, database, and cache, each with a stack of dollar coins for its cost."
    >
      <Tag>a typical platform</Tag>
      {apps.map((cx, i) => {
        const t = 0.2 + i * 0.6;
        return (
          <g key={cx}>
            <Pop t={t}>
              <rect x={cx - 145} y={22} width={290} height={160} rx={10} className="pg-tx" />
              <Doc cx={cx} y={58} s={0.8} />
              <line x1={cx} y1={78} x2={cx} y2={92} className="s-edge" />
            </Pop>
            <Pop t={t + 0.3}>
              <MiniServer cx={cx - 80} cy={116} />
              <Db cx={cx} y={102} h={28} w={30} />
              <MiniCache cx={cx + 80} cy={116} />
            </Pop>
            {[-26, 0, 26].map((dx, k) => (
              <Coin key={dx} cx={cx + dx} cy={160} t={t + 0.7 + k * 0.2} />
            ))}
            <Label x={cx} y={212}>
              app {i + 1} · own infra
            </Label>
          </g>
        );
      })}
    </Scene>
  );
}

// ─── 5. Write in TS, execute in Rust ────────────────────────────────────────────

const RX = { code: 40, artifact: 190, runtime: 340, users: 494 };

/** TypeScript becomes a manifest, and a small Rust core runs the manifest. */
export function RustAleph() {
  const FAST = [0, 1, 2, 3];
  return (
    <Scene aria="Your TypeScript becomes a manifest, a plain JSON file. A small Rust core runs the manifest, and requests from users pass through it quickly one after another.">
      <Card cx={RX.code} />
      <Glyph cx={RX.code}>TS</Glyph>
      <Card cx={RX.artifact} />
      <Glyph cx={RX.artifact}>{"{ }"}</Glyph>
      <Card cx={RX.runtime} strong />
      <Gear cx={RX.runtime} cy={ONE - 8} />
      <text x={RX.runtime} y={ONE + 18} textAnchor="middle" className="pg-glyph" fontSize={13}>
        Rust
      </text>
      <User cx={RX.users} y={ONE} />
      <Dot
        x={RX.code + 32}
        y={ONE}
        t={0.5}
        dx={RX.artifact - 32 - (RX.code + 32)}
        dur={0.6}
        tone="ok"
      />
      <Dot
        x={RX.artifact + 32}
        y={ONE}
        t={1.5}
        dx={RX.runtime - 32 - (RX.artifact + 32)}
        dur={0.6}
        tone="ok"
      />
      {FAST.map((i) => (
        <Dot
          key={i}
          loop
          x={RX.users - 22}
          y={ONE}
          t={0.1 + i * 0.16}
          dx={RX.runtime + 32 - (RX.users - 22)}
          dur={0.64}
        />
      ))}
      <Label x={RX.code}>app.ts</Label>
      <Label x={RX.artifact}>manifest.json</Label>
      <Label x={RX.runtime}>aleph-core · rust</Label>
      <Label x={RX.users}>users</Label>
    </Scene>
  );
}

/** A traditional backend: the code is compiled into a program and a language runtime executes it. */
export function RustTypical() {
  const SLOW = [0, 1];
  return (
    <Scene aria="Your code is compiled into a program, and a language runtime executes it. Requests from users move through it slowly, with pauses, and queue up behind it.">
      <Card cx={RX.code} />
      <Glyph cx={RX.code}>TS</Glyph>
      <Card cx={RX.artifact} />
      <Cube cx={RX.artifact} cy={ONE - 6} />
      <Card cx={RX.runtime} />
      <Gear cx={RX.runtime} cy={ONE - 8} />
      <text x={RX.runtime} y={ONE + 18} textAnchor="middle" className="pg-glyph" fontSize={13}>
        V8
      </text>
      <User cx={RX.users} y={ONE} />
      <Dot x={RX.code + 32} y={ONE} t={0.5} dx={RX.artifact - 32 - (RX.code + 32)} dur={0.6} />
      <Dot
        x={RX.artifact + 32}
        y={ONE}
        t={1.5}
        dx={RX.runtime - 32 - (RX.artifact + 32)}
        dur={0.6}
      />
      {SLOW.map((i) => (
        <Dot
          key={i}
          loop
          x={RX.users - 22}
          y={ONE}
          t={0.1 + i * 1.1}
          dx={RX.runtime + 32 - (RX.users - 22)}
          dur={2.2}
        />
      ))}
      {[0, 1].map((i) => (
        <rect
          key={i}
          x={RX.runtime + 40 + i * 16}
          y={ONE - 7}
          width={11}
          height={14}
          rx={7}
          className="pg-pkt pg-pkt-err pg-at"
          style={vars({ "--t": `${3.4 + i * 0.5}s` })}
        />
      ))}
      <Pop t={3.4}>
        <rect
          x={RX.runtime - 8}
          y={MARK_Y - 4}
          width={5}
          height={18}
          rx={1.5}
          className="pg-pause"
        />
        <rect
          x={RX.runtime + 3}
          y={MARK_Y - 4}
          width={5}
          height={18}
          rx={1.5}
          className="pg-pause"
        />
      </Pop>
      <Label x={RX.code}>app.ts</Label>
      <Label x={RX.artifact}>compiled program</Label>
      <Label x={RX.runtime}>node · v8 runtime</Label>
      <Label x={RX.users}>users</Label>
    </Scene>
  );
}

// ─── 6. Built in the language ───────────────────────────────────────────────────

/** The icon for one thing the platform generates from the manifest, centered on (cx, cy). */
function FeatureIcon({ name, cx, cy }: { name: string; cx: number; cy: number }) {
  switch (name) {
    case "auth":
      return <Keyhole cx={cx} cy={cy} />;
    case "logs":
      return (
        <g>
          {[-9, 0, 9].map((dy) => (
            <line
              key={dy}
              x1={cx - 13}
              y1={cy + dy}
              x2={cx + 13 - Math.abs(dy)}
              y2={cy + dy}
              className="pg-line"
            />
          ))}
        </g>
      );
    case "workflows":
      return (
        <g>
          <line x1={cx - 18} y1={cy} x2={cx + 18} y2={cy} className="pg-line" />
          {[-18, 0, 18].map((dx) => (
            <circle key={dx} cx={cx + dx} cy={cy} r={5} className="pg-env" />
          ))}
        </g>
      );
    case "db":
      return <Db cx={cx} y={cy - 14} h={28} w={30} />;
    case "queues":
      return (
        <g>
          {[-17, -3, 11].map((dx) => (
            <rect key={dx} x={cx + dx} y={cy - 7} width={9} height={14} rx={2} className="pg-env" />
          ))}
          <polyline
            points={`${cx + 23},${cy - 5} ${cx + 28},${cy} ${cx + 23},${cy + 5}`}
            className="pg-line"
          />
        </g>
      );
    case "files":
      return (
        <g>
          <polygon
            points={`${cx - 10},${cy - 13} ${cx + 4},${cy - 13} ${cx + 10},${cy - 7} ${cx + 10},${cy + 13} ${cx - 10},${cy + 13}`}
            className="pg-env"
          />
          <polyline
            points={`${cx + 4},${cy - 13} ${cx + 4},${cy - 7} ${cx + 10},${cy - 7}`}
            className="pg-line"
          />
        </g>
      );
    case "mcp":
      return (
        <g>
          {[
            [-16, -9],
            [16, -9],
            [0, 14],
          ].map(([dx, dy]) => (
            <line
              key={`${dx}${dy}`}
              x1={cx}
              y1={cy}
              x2={cx + (dx as number)}
              y2={cy + (dy as number)}
              className="pg-line"
            />
          ))}
          {[
            [-16, -9],
            [16, -9],
            [0, 14],
          ].map(([dx, dy]) => (
            <circle
              key={`c${dx}${dy}`}
              cx={cx + (dx as number)}
              cy={cy + (dy as number)}
              r={4.5}
              className="pg-env"
            />
          ))}
          <circle cx={cx} cy={cy} r={6} className="pg-shape" />
        </g>
      );
    case "openapi":
      return (
        <text x={cx} y={cy + 6} textAnchor="middle" className="pg-glyph" fontSize={18}>
          {"{ }"}
        </text>
      );
    default:
      return (
        <g>
          <rect x={cx - 15} y={cy - 13} width={30} height={26} rx={4} className="pg-env" />
          <polygon
            points={`${cx - 4},${cy - 6} ${cx + 7},${cy} ${cx - 4},${cy + 6}`}
            className="pg-shape-fill"
          />
        </g>
      );
  }
}

/** What the platform generates from the manifest, and the one-sentence benefit of each. */
const BUILT_IN = [
  {
    id: "auth",
    name: "auth",
    lines: ["Routes are private by", "default and roles are", "declared, not hand-wired."],
  },
  {
    id: "db",
    name: "database",
    lines: [
      "A transactional store with",
      "indexes you declare, so no",
      "ORM or migrations to run.",
    ],
  },
  {
    id: "logs",
    name: "logs",
    lines: ["The platform logs requests", "and jobs, so there is no", "logger to set up."],
  },
  {
    id: "workflows",
    name: "workflows",
    lines: ["Multi-step work waits for", "days and survives crashes", "with no job server."],
  },
  {
    id: "queues",
    name: "queues",
    lines: ["Retries, backoff and a", "dead-letter handler are", "declared, not coded."],
  },
  {
    id: "files",
    name: "files",
    lines: ["Signed uploads with size", "and type limits, reachable", "only by your app."],
  },
  {
    id: "mcp",
    name: "MCP server",
    lines: ["Agents can call your routes", "as tools, generated from", "the same manifest."],
  },
  {
    id: "openapi",
    name: "OpenAPI spec",
    lines: ["An exact API spec that", "stays in sync, because it", "is derived, not written."],
  },
  {
    id: "swagger",
    name: "Swagger docs",
    lines: ["Interactive docs to try", "every route, rendered from", "the OpenAPI spec."],
  },
];

const BIG_H = 570;

/**
 * Your code becomes a manifest, and a platform that supports the manifest generates everything
 * else from it: auth, database, logs, workflows, queues, files, an MCP server, an OpenAPI spec,
 * and Swagger docs. Each one appears with a sentence on why it matters.
 */
function BuiltInDesktop() {
  const colX = [330, 550, 770];
  const rowY = [26, 194, 362];
  return (
    <Scene
      wide
      big
      className="pg-scene-desktop"
      w={1000}
      h={BIG_H}
      aria="Your code becomes a manifest. A platform that supports the manifest generates auth, a database, logs, workflows, queues, files, an MCP server, an OpenAPI spec, and Swagger docs from it, each with a short description of its benefit."
    >
      <Pop t={0.2}>
        <Doc cx={60} y={285} s={1.5} />
      </Pop>
      <line x1={100} y1={285} x2={165} y2={285} className="s-edge" />
      <Dot x={100} y={285} t={0.7} dx={62} dur={0.6} tone="ok" />
      <Pop t={1.2}>
        <rect x={165} y={215} width={80} height={140} rx={8} className="s-card s-strong-card" />
        <text x={205} y={291} textAnchor="middle" className="pg-glyph" fontSize={18}>
          {"{ }"}
        </text>
      </Pop>
      <line x1={245} y1={285} x2={316} y2={285} className="s-edge" />
      <Dot x={245} y={285} t={1.9} dx={66} dur={0.6} tone="ok" />
      <Pop t={2.4}>
        <rect x={316} y={10} width={674} height={520} rx={10} className="pg-tx" />
      </Pop>
      {BUILT_IN.map((f, i) => {
        const x = colX[i % 3] as number;
        const y = rowY[Math.floor(i / 3)] as number;
        const t = 3 + i * 0.9;
        const cx = x + 102;
        return (
          <Pop key={f.id} t={t}>
            <rect x={x} y={y} width={205} height={152} rx={8} className="pg-tile" />
            <FeatureIcon name={f.id} cx={cx} cy={y + 34} />
            <text x={cx} y={y + 74} textAnchor="middle" className="pg-name">
              {f.name}
            </text>
            {f.lines.map((line, k) => (
              <text key={line} x={cx} y={y + 98 + k * 16} textAnchor="middle" className="pg-desc">
                {line}
              </text>
            ))}
          </Pop>
        );
      })}
      <Label x={60} y={BIG_H - 8}>
        your code
      </Label>
      <Label x={205} y={BIG_H - 8}>
        manifest
      </Label>
      <Label x={653} y={BIG_H - 8}>
        generated from the manifest
      </Label>
    </Scene>
  );
}

/** The same scene top to bottom, for screens too narrow for a grid of cards. */
function BuiltInMobile() {
  const CARD_H = 108;
  const STEP = 118;
  const Y0 = 146;
  const total = Y0 + (BUILT_IN.length - 1) * STEP + CARD_H + 16;
  return (
    <Scene
      big
      className="pg-scene-mobile"
      w={360}
      h={total + 16}
      aria="Your code becomes a manifest. A platform that supports the manifest generates auth, a database, logs, workflows, queues, files, an MCP server, an OpenAPI spec, and Swagger docs from it, each with a short description of its benefit."
    >
      <Pop t={0.2}>
        <Doc cx={60} y={54} />
      </Pop>
      <line x1={80} y1={54} x2={160} y2={54} className="s-edge" />
      <Dot x={80} y={54} t={0.7} dx={74} dur={0.6} tone="ok" />
      <Pop t={1.2}>
        <rect x={160} y={24} width={80} height={60} rx={8} className="s-card s-strong-card" />
        <text x={200} y={60} textAnchor="middle" className="pg-glyph" fontSize={18}>
          {"{ }"}
        </text>
      </Pop>
      <path d="M240 54 H 330 V 130" className="s-edge" />
      <Dot x={240} y={54} t={1.9} dx={86} dur={0.6} tone="ok" />
      <Dot x={330} y={54} t={2.4} dx={0} dy={72} dur={0.4} tone="ok" />
      <Label x={60} y={102}>
        your code
      </Label>
      <Label x={200} y={102}>
        manifest
      </Label>
      <Pop t={2.4}>
        <rect x={10} y={130} width={340} height={total - 130} rx={10} className="pg-tx" />
        <text x={22} y={123} className="pg-tag">
          generated from the manifest
        </text>
      </Pop>
      {BUILT_IN.map((f, i) => {
        const y = Y0 + i * STEP;
        return (
          <Pop key={f.id} t={3 + i * 0.9}>
            <rect x={24} y={y} width={312} height={CARD_H} rx={8} className="pg-tile" />
            <FeatureIcon name={f.id} cx={68} cy={y + CARD_H / 2} />
            <text x={110} y={y + 34} className="pg-name">
              {f.name}
            </text>
            {f.lines.map((line, k) => (
              <text key={line} x={110} y={y + 58 + k * 16} className="pg-desc">
                {line}
              </text>
            ))}
          </Pop>
        );
      })}
    </Scene>
  );
}

/** One scene on wide screens, the other on narrow ones; CSS picks. */
export function BuiltInScene() {
  return (
    <div className="pg-builtin">
      <BuiltInDesktop />
      <BuiltInMobile />
    </div>
  );
}
