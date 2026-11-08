import type { CSSProperties, ReactNode } from "react";
import { Visible } from "@/components/Visible";

/**
 * The small animations above each playground code pane. Every scene is built from the same
 * pieces on the same stage: callers on the left, tall cards for the things they reach (a gate, a
 * server, a database, a mailbox), and a footer that names each one. A wide pill is a big request,
 * red is what goes wrong and green is what is accepted. A scene plays once (the playground
 * remounts it to replay) and ends on a still that says the same thing, which is also what people
 * who prefer reduced motion see. Times are seconds from the start; see globals.css, "Playground".
 */

const vars = (o: Record<string, string | number>) => o as unknown as CSSProperties;
type Tone = "ok" | "err" | "plain";

// The stage: every card is as tall as the stage; labels sit on one footer line.
const TOP = 14;
const H = 120;
const LABEL_Y = 160;
const USERS = 40;
const MID = 267;
const DB = 494;
/** Where a request ends up when it stops at a card's left edge (minus its own width). */
const reach = (edge: number, x: number, w: number) => edge - 4 - (x + w);

function Scene({ aria, children }: { aria: string; children: ReactNode }) {
  return (
    <figure className="demo pg-scene">
      <Visible>
        <svg viewBox="0 0 560 172" className="scene" role="img" aria-label={aria}>
          {children}
        </svg>
      </Visible>
    </figure>
  );
}

// ─── Shapes ─────────────────────────────────────────────────────────────────────

function Label({ x, children }: { x: number; children: ReactNode }) {
  return (
    <text x={x} y={LABEL_Y} textAnchor="middle" className="pg-label">
      {children}
    </text>
  );
}

/** A caller: a solid circle when signed in, a dashed one when anonymous. */
function User({ y, anon }: { y: number; anon?: boolean }) {
  const cls = anon ? "pg-shape pg-anon" : "pg-shape";
  return (
    <g>
      <circle cx={USERS} cy={y} r={14} className={cls} />
      <circle cx={USERS} cy={y - 3} r={4} className={anon ? "pg-anon-fill" : "pg-shape-fill"} />
      <path d={`M${USERS - 7} ${y + 8} a7 5 0 0 1 14 0`} className={cls} />
    </g>
  );
}

/** A piece of code: a page with lines. */
function Doc({ y }: { y: number }) {
  return (
    <g>
      <rect x={USERS - 19} y={y - 24} width={38} height={48} rx={5} className="s-card" />
      {[12, 21, 30, 39].map((dy) => (
        <line
          key={dy}
          x1={USERS - 11}
          y1={y - 24 + dy}
          x2={USERS + 11 - (dy % 18)}
          y2={y - 24 + dy}
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

/** A check that every request goes through: a gate with a keyhole. */
function Gate({ cx }: { cx: number }) {
  const cy = TOP + H / 2;
  return (
    <g>
      <Card cx={cx} strong />
      <circle cx={cx} cy={cy - 4} r={6} className="pg-shape" />
      <line x1={cx} y1={cy + 2} x2={cx} y2={cy + 14} className="pg-shape" />
    </g>
  );
}

/** Your server: a card with a few bars. */
function Server({ cx }: { cx: number }) {
  return (
    <g>
      <Card cx={cx} />
      <line x1={cx - 16} y1={TOP + 14} x2={cx + 16} y2={TOP + 14} className="s-edge" />
      <line x1={cx - 16} y1={TOP + 25} x2={cx + 16} y2={TOP + 25} className="s-edge" />
      <circle cx={cx + 16} cy={TOP + H - 12} r={2.5} className="pg-shape-fill" />
    </g>
  );
}

/** A database: a cylinder. */
function Db({ cx, w = 64 }: { cx: number; w?: number }) {
  const r = 9;
  const x = cx - w / 2;
  return (
    <g>
      <path
        d={`M${x} ${TOP + r} v${H - 2 * r} a${w / 2} ${r} 0 0 0 ${w} 0 v${-(H - 2 * r)}`}
        className="s-card"
      />
      <ellipse cx={cx} cy={TOP + r} rx={w / 2} ry={r} className="s-card" />
    </g>
  );
}

/** A transaction: a dashed boundary that things happen inside together. */
function Tx({ cx }: { cx: number }) {
  return <rect x={cx - 50} y={TOP} width={100} height={H} rx={8} className="pg-tx" />;
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

/** A mailbox: a card holding an envelope per lane. */
function Mailbox({ cx, lanes }: { cx: number; lanes: number[] }) {
  return (
    <g>
      <Card cx={cx} w={64} />
      {lanes.map((y) => (
        <Envelope key={y} cx={cx} cy={y} />
      ))}
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
function Mark({ cx, cy, ok, t }: { cx: number; cy: number; ok?: boolean; t: number }) {
  return (
    <g className="pg-at" style={vars({ "--t": `${t}s` })}>
      <circle
        cx={cx}
        cy={cy}
        r={12}
        className={ok ? "pg-mark-bg pg-mark-bg-ok" : "pg-mark-bg pg-mark-bg-err"}
      />
      {ok ? (
        <polyline
          points={`${cx - 5},${cy} ${cx - 1.5},${cy + 4} ${cx + 5.5},${cy - 4}`}
          className="pg-mark pg-mark-ok"
        />
      ) : (
        <>
          <line
            x1={cx - 4.5}
            y1={cy - 4.5}
            x2={cx + 4.5}
            y2={cy + 4.5}
            className="pg-mark pg-mark-err"
          />
          <line
            x1={cx + 4.5}
            y1={cy - 4.5}
            x2={cx - 4.5}
            y2={cy + 4.5}
            className="pg-mark pg-mark-err"
          />
        </>
      )}
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
  lane,
  w,
  tone = "plain",
  t,
  dx,
  dy = 0,
  dur = 1,
  o = 0,
  from = USERS + 24,
}: {
  lane: number;
  w: number;
  tone?: Tone;
  t: number;
  dx: number;
  dy?: number;
  dur?: number;
  o?: number;
  from?: number;
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
}: {
  x: number;
  y: number;
  t: number;
  dx: number;
  dy?: number;
  dur?: number;
  tone?: Tone;
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
          "--o": 0,
        })}
      >
        <circle r={4.5} className={`pg-dot pg-dot-${tone}`} />
      </g>
    </g>
  );
}

const L1 = 40;
const L2 = 108;
const ONE = TOP + H / 2;
const MID_L = MID - 28;
const DB_L = DB - 32;

// ─── 1. Bounded ─────────────────────────────────────────────────────────────────

/** A request for far too much stops at the gate; a bounded one goes through. */
export function BoundedAleph() {
  return (
    <Scene aria="A very large request is stopped at the gate and never runs. A small, bounded request passes through to the database.">
      <Doc y={ONE} />
      <Gate cx={MID} />
      <Db cx={DB} />
      <Pkt lane={ONE} w={64} tone="err" t={1.4} dx={reach(MID_L, USERS + 24, 64)} dur={1} />
      {flashCard(MID, 2.5, "err")}
      <Mark cx={MID} cy={ONE} t={2.5} />
      <Pkt lane={ONE} w={26} tone="ok" t={4.6} dx={reach(DB_L, USERS + 24, 26)} dur={1.4} />
      <Row cx={DB} y={ONE - 16} t={6.1} tone="ok" />
      <Row cx={DB} y={ONE - 2} t={6.25} tone="ok" />
      <Row cx={DB} y={ONE + 12} t={6.4} tone="ok" />
      <Mark cx={DB} cy={ONE + 42} ok t={6.5} />
      <Label x={USERS}>app.ts</Label>
      <Label x={MID}>aleph check</Label>
      <Label x={DB}>db</Label>
    </Scene>
  );
}

/** The same request is accepted, and a flood of queries wears the database out. */
export function BoundedNode() {
  const dots = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
  return (
    <Scene aria="The same large request is accepted. A flood of queries streams into the database, its load climbs, and it fails.">
      <User y={ONE} />
      <Server cx={MID} />
      <Db cx={DB} />
      <rect x={DB - 20} y={ONE + 20} width={40} height={7} rx={3} className="s-track" />
      <rect
        x={DB - 20}
        y={ONE + 20}
        width={40}
        height={7}
        rx={3}
        className="pg-load pg-grow"
        style={vars({ "--t": "1.9s", "--d": "3.2s" })}
      />
      <Pkt lane={ONE} w={64} t={0.4} dx={reach(MID_L, USERS + 24, 64)} dur={0.8} />
      <Mark cx={MID} cy={ONE + 22} ok t={1.3} />
      {dots.map((i) => (
        <Dot
          key={i}
          x={MID + 32}
          y={ONE}
          t={1.7 + i * 0.3}
          dx={DB_L - 4 - (MID + 32)}
          dy={-16 + i * 3.5}
        />
      ))}
      <Mark cx={DB} cy={ONE - 22} t={5.2} />
      <Label x={USERS}>users</Label>
      <Label x={MID}>server</Label>
      <Label x={DB}>db</Label>
    </Scene>
  );
}

// ─── 2. Exactly once ────────────────────────────────────────────────────────────

/** Writes and effects commit together; a crash leaves no half-done work behind. */
export function ExactlyAleph() {
  const TX_L = MID - 50;
  const lane = (y: number, crash: boolean) => (
    <g>
      <User y={y} />
      <Pkt lane={y} w={30} t={0.4} dx={reach(TX_L, USERS + 24, 30)} dur={0.8} />
      <Row cx={MID - 24} y={y - 5} t={1.3} />
      <g className="pg-at" style={vars({ "--t": "1.6s" })}>
        <Envelope cx={MID + 26} cy={y} w={32} />
      </g>
      {crash ? (
        <>
          <Flash x={TX_L} y={y - 24} w={100} h={48} tone="err" t={2.3} />
          <Mark cx={MID + 78} cy={y} t={2.3} />
        </>
      ) : (
        <>
          <Mark cx={MID + 78} cy={y} ok t={2.3} />
          <Dot x={MID + 102} y={y} t={2.9} dx={DB - 32 - 4 - (MID + 102)} dur={0.7} tone="ok" />
          <circle
            cx={DB + 18}
            cy={y - 12}
            r={6}
            className="pg-dot pg-dot-ok pg-at"
            style={vars({ "--t": "3.6s" })}
          />
        </>
      )}
    </g>
  );
  return (
    <Scene aria="A request writes a row and queues a mail inside one transaction. When it commits the mail is sent once. When it crashes, both are rolled back and nothing is sent.">
      <Tx cx={MID} />
      <Mailbox cx={DB} lanes={[L1, L2]} />
      {lane(L1, false)}
      {lane(L2, true)}
      <Label x={USERS}>users</Label>
      <Label x={MID}>transaction</Label>
      <Label x={DB}>mail</Label>
    </Scene>
  );
}

/** The write and the mail are separate steps; a crash between them loses the mail. */
export function ExactlyNode() {
  const SERVER = 170;
  const DBX = 340;
  const SERVER_L = SERVER - 28;
  return (
    <Scene aria="A server saves a row, then crashes before it sends the mail. The row exists and the mail never goes out.">
      <User y={ONE} />
      <path
        d={`M${SERVER} ${TOP + H} C ${SERVER + 20} ${TOP + H + 22}, ${DB - 20} ${TOP + H + 22}, ${DB} ${TOP + H}`}
        className="pg-ghost pg-at"
        style={vars({ "--t": "2.9s" })}
      />
      <Server cx={SERVER} />
      <Db cx={DBX} />
      <Mailbox cx={DB} lanes={[ONE]} />
      <Pkt lane={ONE} w={30} t={0.3} dx={reach(SERVER_L, USERS + 24, 30)} dur={0.7} />
      <Dot x={SERVER + 32} y={ONE} t={1.3} dx={DBX - 32 - 4 - (SERVER + 32)} dur={0.8} />
      <Row cx={DBX} y={ONE - 5} t={2.2} tone="ok" />
      <Mark cx={DBX} cy={ONE + 30} ok t={2.4} />
      {flashCard(SERVER, 2.9, "err")}
      <Mark cx={SERVER} cy={ONE} t={2.9} />
      <Label x={USERS}>users</Label>
      <Label x={SERVER}>server</Label>
      <Label x={DBX}>db</Label>
      <Label x={DB}>mail</Label>
    </Scene>
  );
}

// ─── 3. Private by default ──────────────────────────────────────────────────────

/** A caller with no session is turned away at the gate; a signed-in one goes through. */
export function PrivateAleph() {
  return (
    <Scene aria="A request from an anonymous caller is stopped at the gate. A request from a signed-in caller passes through to the data.">
      <User y={L1} anon />
      <User y={L2} />
      <Gate cx={MID} />
      <Db cx={DB} />
      <Pkt lane={L1} w={30} t={0.5} dx={reach(MID_L, USERS + 24, 30)} dur={1.2} />
      {flashCard(MID, 1.8, "err")}
      <Mark cx={MID} cy={L1} t={1.8} />
      <Pkt lane={L2} w={30} tone="ok" t={2.4} dx={reach(DB_L, USERS + 24, 30)} dur={1.7} />
      <Mark cx={MID} cy={L2} ok t={3.1} />
      <Row cx={DB} y={L2 - 14} t={4.2} tone="ok" />
      <Row cx={DB} y={L2} t={4.35} tone="ok" />
      <Row cx={DB} y={L2 + 14} t={4.5} tone="ok" />
      <Label x={USERS}>users</Label>
      <Label x={MID}>sign-in check</Label>
      <Label x={DB}>db</Label>
    </Scene>
  );
}

/** A route nobody guarded hands its data to anyone who asks. */
export function PrivateNode() {
  return (
    <Scene aria="A request from an anonymous caller goes straight through an unguarded route, and rows from the database flow back to the caller.">
      <User y={ONE} anon />
      <Server cx={MID} />
      <Db cx={DB} />
      <Pkt lane={ONE} w={30} t={0.4} dx={reach(MID_L, USERS + 24, 30)} dur={1} />
      <Dot x={MID + 32} y={ONE} t={1.5} dx={DB_L - 4 - (MID + 32)} dur={0.8} />
      <Pkt
        lane={ONE - 14}
        w={44}
        tone="err"
        t={2.4}
        dx={USERS + 24 - (DB - 22)}
        dur={1.4}
        o={1}
        from={DB - 22}
      />
      <Pkt
        lane={ONE}
        w={44}
        tone="err"
        t={2.7}
        dx={USERS + 24 - (DB - 22)}
        dur={1.4}
        o={1}
        from={DB - 22}
      />
      <Pkt
        lane={ONE + 14}
        w={44}
        tone="err"
        t={3}
        dx={USERS + 24 - (DB - 22)}
        dur={1.4}
        o={1}
        from={DB - 22}
      />
      <Mark cx={USERS} cy={ONE + 34} t={4.4} />
      <Label x={USERS}>users</Label>
      <Label x={MID}>server</Label>
      <Label x={DB}>db</Label>
    </Scene>
  );
}

// ─── 4. Less to write ───────────────────────────────────────────────────────────

/** A retry of the same request is answered from what was stored, and writes nothing. */
export function LessAleph() {
  return (
    <Scene aria="A request is stored once. A retry of the same request is answered from the stored response, and nothing more is written.">
      <User y={L1} />
      <User y={L2} />
      <Gate cx={MID} />
      <Db cx={DB} />
      <Pkt lane={L1} w={36} t={0.5} dx={reach(DB_L, USERS + 24, 36)} dur={1.5} />
      <Row cx={DB} y={L1 - 5} t={2.1} tone="ok" />
      <Mark cx={DB} cy={L1 + 28} ok t={2.3} />
      <Pkt lane={L2} w={36} t={3} dx={reach(MID_L, USERS + 24, 36)} dur={1} />
      {flashCard(MID, 4, "ok")}
      <Pkt
        lane={L2}
        w={36}
        tone="ok"
        t={4.1}
        dx={-(MID_L - 4 - 36 - (USERS + 24))}
        dur={1}
        o={1}
        from={MID_L - 4 - 36}
      />
      <Label x={USERS}>users</Label>
      <Label x={MID}>idempotency</Label>
      <Label x={DB}>db</Label>
    </Scene>
  );
}

/** Two retries race past the check and both write. */
export function LessNode() {
  return (
    <Scene aria="Two retries of the same request both get past the server's check before either result is stored, so both are written.">
      <User y={L1} />
      <User y={L2} />
      <Server cx={MID} />
      <Db cx={DB} />
      <Pkt lane={L1} w={36} t={0.5} dx={reach(MID_L, USERS + 24, 36)} dur={0.9} />
      <Pkt lane={L2} w={36} t={0.65} dx={reach(MID_L, USERS + 24, 36)} dur={0.9} />
      <Dot x={MID + 32} y={L1} t={2} dx={DB_L - 4 - (MID + 32)} dur={0.8} />
      <Dot x={MID + 32} y={L2} t={2.15} dx={DB_L - 4 - (MID + 32)} dy={-40} dur={0.8} />
      <Row cx={DB} y={ONE - 22} t={2.9} tone="err" />
      <Row cx={DB} y={ONE - 6} t={3.2} tone="err" />
      <Mark cx={DB} cy={ONE + 34} t={3.6} />
      <Label x={USERS}>users</Label>
      <Label x={MID}>server</Label>
      <Label x={DB}>db</Label>
    </Scene>
  );
}
