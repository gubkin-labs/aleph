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
const WIDE: Stage = { w: 760, users: 40, mid: 380, db: 720 };

/** How far a request starting at `from` with width `w` travels to stop at `edge`. */
const reach = (edge: number, from: number, w: number) => edge - 4 - (from + w);

function Scene({
  aria,
  w = 560,
  h = 172,
  wide,
  children,
}: {
  aria: string;
  w?: number;
  h?: number;
  wide?: boolean;
  children: ReactNode;
}) {
  return (
    <figure className={wide ? "demo pg-scene pg-scene-wide" : "demo pg-scene"}>
      <Visible>
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
function Tag({ x = 14, t, children }: { x?: number; t?: number; children: ReactNode }) {
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
function Ship({ st, gate }: { st: Stage; gate: number }) {
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
      <Mark cx={st.mid} ok t={10.9} />
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
}: {
  x: number;
  y: number;
  t: number;
  dx: number;
  dy?: number;
  dur?: number;
  tone?: Tone;
  o?: number;
}) {
  return (
    <g transform={`translate(${x} ${y})`}>
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

const GATE = 210;

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
      <Ship st={st} gate={GATE} />

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
  const lane = (y: number, t0: number) => (
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
          <Flash x={TXL} y={y - 28} w={TXR - TXL} h={56} tone="err" t={t0 + 2} />
          <Mark cx={DB_X + 20} cy={y - 14} t={t0 + 2} k={0.7} />
          <Mark cx={MAIL_X + 22} cy={y - 14} t={t0 + 2} k={0.7} />
        </>
      ) : (
        <>
          <Flash x={TXL} y={y - 28} w={120} h={56} tone="ok" t={t0 + 1.7} />
          <Mark cx={DB_X + 20} cy={y - 14} ok t={t0 + 1.7} k={0.7} />
          <Mark cx={(TXL + 120 + TXR - 120) / 2} cy={y} t={t0 + 2.4} k={0.8} />
          <Flash x={TXR - 120} y={y - 28} w={120} h={56} tone="err" t={t0 + 3} />
          <Mark cx={MAIL_X + 22} cy={y - 14} t={t0 + 3} k={0.7} />
        </>
      )}
    </g>
  );
  return (
    <Scene
      aria={
        atomic
          ? "Two users send requests at the same time. Each request is one transaction holding a database insert and an email. When it fails, both the insert and the email are rolled back."
          : "Two users send requests at the same time. Each request is two separate transactions: a database insert, then an email. The insert is committed, the server fails, and the email is never sent."
      }
    >
      {lane(L1, 0.3)}
      {lane(L2, 0.5)}
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
const WW = 760;

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
      <rect x={30} y={22} width={700} height={160} rx={10} className="pg-tx" />
      {docs.map((i) => {
        const cx = 95 + i * 114;
        return (
          <Pop key={cx} t={0.3 + i * 0.25}>
            <Doc cx={cx} y={58} s={0.8} />
            <line x1={cx} y1={78} x2={cx} y2={92} className="s-edge" />
          </Pop>
        );
      })}
      <Pop t={1.9}>
        <line x1={95} y1={92} x2={665} y2={92} className="s-edge" />
        <line x1={380} y1={92} x2={380} y2={98} className="s-edge" />
        <MiniServer cx={260} cy={116} />
        <Db cx={380} y={102} h={28} w={30} />
        <MiniCache cx={500} cy={116} />
      </Pop>
      <Coin cx={380} cy={160} t={2.7} />
      <Label x={380} y={212}>
        6 apps · one runtime · shared infra
      </Label>
    </Scene>
  );
}

/** Typical platforms: every app brings its own runtime, database, and cache, and its own bill. */
export function SharedTypical() {
  const apps = [130, 380, 630];
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
              <rect x={cx - 105} y={22} width={210} height={160} rx={10} className="pg-tx" />
              <Doc cx={cx} y={58} s={0.8} />
              <line x1={cx} y1={78} x2={cx} y2={92} className="s-edge" />
            </Pop>
            <Pop t={t + 0.3}>
              <MiniServer cx={cx - 62} cy={116} />
              <Db cx={cx} y={102} h={28} w={30} />
              <MiniCache cx={cx + 62} cy={116} />
            </Pop>
            {[-24, 0, 24].map((dx, k) => (
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
  const dots = [0, 1, 2, 3, 4, 5];
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
      {dots.map((i) => (
        <Dot
          key={i}
          x={RX.users - 22}
          y={ONE}
          t={2.5 + i * 0.3}
          dx={RX.runtime + 32 - (RX.users - 22)}
          dur={0.45}
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
  const dots = [0, 1, 2, 3];
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
      {dots.map((i) => (
        <Dot
          key={i}
          x={RX.users - 22}
          y={ONE}
          t={2.5 + i * 0.9}
          dx={RX.runtime + 32 - (RX.users - 22)}
          dur={1.5}
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

// ─── 6. Platform agnostic ───────────────────────────────────────────────────────

const OUTPUTS = ["MCP server", "OpenAPI spec", "Swagger docs"];
const OUT_Y = [18, 80, 142];

/** A feature's name on a card: with a check when it is built in, with a cog when it is a library. */
function Output({
  x,
  y,
  w = 200,
  name,
  t,
  ok,
}: {
  x: number;
  y: number;
  w?: number;
  name: string;
  t: number;
  ok?: boolean;
}) {
  return (
    <Pop t={t}>
      <rect
        x={x}
        y={y}
        width={w}
        height={44}
        rx={7}
        className={ok ? "s-card s-strong-card" : "s-card"}
      />
      <text x={x + 14} y={y + 26} className="pg-feature">
        {name}
      </text>
      {ok ? (
        <Mark cx={x + w - 22} cy={y + 22} ok t={0} k={0.75} />
      ) : (
        <Gear cx={x + w - 22} cy={y + 22} r={7} />
      )}
    </Pop>
  );
}

/**
 * With Aleph: the manifest goes into a platform, and the platform gives you MCP, OpenAPI, and
 * Swagger docs generated from it, plus auth, logs, and a database it already runs.
 */
export function AgnosticAleph() {
  return (
    <Scene
      wide
      w={WW}
      h={SH}
      aria="A manifest goes into a platform. The platform generates an MCP server, an OpenAPI spec, and Swagger docs from it, and provides auth, logs, and a database."
    >
      <Pop t={0.2}>
        <rect x={30} y={46} width={64} height={110} rx={8} className="s-card" />
        <Glyph cx={62} size={17}>
          {"{ }"}
        </Glyph>
      </Pop>
      <line x1={94} y1={101} x2={150} y2={101} className="s-edge" />
      <Dot x={94} y={101} t={0.8} dx={52} dur={0.6} tone="ok" />
      <Pop t={1.2}>
        <rect x={150} y={18} width={320} height={170} rx={10} className="s-card s-strong-card" />
        <g>
          <rect x={186} y={72} width={44} height={56} rx={6} className="pg-env" />
          <Keyhole cx={208} cy={100} />
          <text x={208} y={146} textAnchor="middle" className="pg-tiny">
            auth
          </text>
        </g>
        <g>
          <rect x={288} y={72} width={44} height={56} rx={6} className="pg-env" />
          {[86, 98, 110].map((y) => (
            <line key={y} x1={297} y1={y} x2={323} y2={y} className="pg-line" />
          ))}
          <text x={310} y={146} textAnchor="middle" className="pg-tiny">
            logs
          </text>
        </g>
        <g>
          <Db cx={412} y={72} h={56} w={44} />
          <text x={412} y={146} textAnchor="middle" className="pg-tiny">
            db
          </text>
        </g>
      </Pop>
      {OUTPUTS.map((name, i) => {
        const y = (OUT_Y[i] as number) + 22;
        return (
          <g key={name}>
            <line x1={470} y1={y} x2={530} y2={y} className="s-edge" />
            <Dot x={470} y={y} t={2 + i * 0.5} dx={56} dur={0.5} tone="ok" />
            <Output x={530} y={y - 22} name={name} t={2.5 + i * 0.5} ok />
          </g>
        );
      })}
      <Label x={62} y={212}>
        manifest
      </Label>
      <Label x={310} y={212}>
        platform
      </Label>
      <Label x={630} y={212}>
        generated from it
      </Label>
    </Scene>
  );
}

const LIBS = [
  "mcp sdk",
  "openapi lib",
  "swagger ui",
  "passport · auth",
  "pino · logs",
  "orm + pg · db",
];

/** A traditional backend: each of those is another library to choose, install, wire up, and keep updated. */
export function AgnosticTypical() {
  return (
    <Scene
      wide
      w={WW}
      h={SH}
      aria="A traditional backend needs a separate library for MCP, OpenAPI, Swagger docs, auth, logs, and the database, each one installed and wired up by hand."
    >
      <Pop t={0.2}>
        <rect x={30} y={46} width={64} height={110} rx={8} className="s-card" />
        <Glyph cx={62} size={17}>
          TS
        </Glyph>
      </Pop>
      <line
        x1={94}
        y1={101}
        x2={150}
        y2={101}
        className="pg-ghost pg-at"
        style={vars({ "--t": "0.8s" })}
      />
      <Pop t={1}>
        <rect x={150} y={18} width={560} height={170} rx={10} className="pg-tx" />
      </Pop>
      {LIBS.map((name, i) => {
        const col = i % 3;
        const row = Math.floor(i / 3);
        return (
          <Output
            key={name}
            x={170 + col * 180}
            y={40 + row * 70}
            w={170}
            name={name}
            t={1.4 + i * 0.4}
          />
        );
      })}
      <Label x={62} y={212}>
        your server
      </Label>
      <Label x={430} y={212}>
        6 libraries to wire up and keep updated
      </Label>
    </Scene>
  );
}
