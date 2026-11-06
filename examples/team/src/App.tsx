import {
  type CallResult,
  Inspector,
  ResultView,
  useAleph,
  useQuery,
} from "@aleph-examples/inspector/react";
import { useState } from "react";

interface Member {
  memberId: string;
  email: string;
  role: "owner" | "admin" | "member";
}

interface Members {
  seats: number;
  page: { items: Member[]; cursor: string | null };
}

const ORG = "/orgs/acme";

export function App() {
  const { call } = useAleph();
  const [last, setLast] = useState<CallResult>();
  const [orgName, setOrgName] = useState("Acme");
  const [plan, setPlan] = useState<"free" | "pro">("free");
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<Member["role"]>("member");
  const [inviteKey, setInviteKey] = useState(() => crypto.randomUUID());
  const [lookup, setLookup] = useState("");
  const members = useQuery<Members>(`${ORG}/members`);

  const run = async (result: Promise<CallResult>) => setLast(await result);

  return (
    <div className="al-layout">
      <main className="al-main">
        <h1>Team</h1>
        <p>
          Org roles and API keys come from the caller (pick one in the inspector). Emails are unique
          per org ignoring case, seats are limited by plan, and invites are idempotent.
        </p>

        <section className="al-card">
          <h3>1. Org (owners only)</h3>
          <div className="al-row">
            <input
              aria-label="Org name"
              value={orgName}
              onChange={(e) => setOrgName(e.target.value)}
            />
            <select
              aria-label="Plan"
              value={plan}
              onChange={(e) => setPlan(e.target.value as "free" | "pro")}
            >
              <option value="free">free (3 seats)</option>
              <option value="pro">pro (500 seats)</option>
            </select>
            <button
              type="button"
              onClick={() => run(call("PUT", ORG, { body: { name: orgName, plan } }))}
            >
              Save org
            </button>
          </div>
        </section>

        <section className="al-card">
          <h3>2. Invite (owners and admins)</h3>
          <div className="al-row">
            <input
              aria-label="Email"
              placeholder="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
            />
            <select
              aria-label="Role"
              value={role}
              onChange={(e) => setRole(e.target.value as Member["role"])}
            >
              <option value="member">member</option>
              <option value="admin">admin</option>
              <option value="owner">owner</option>
            </select>
            <button
              type="button"
              onClick={() =>
                run(
                  call("POST", `${ORG}/members`, {
                    body: { email, role },
                    idempotencyKey: inviteKey,
                  }),
                )
              }
            >
              Invite
            </button>
            <button type="button" onClick={() => setInviteKey(crypto.randomUUID())}>
              New idempotency key
            </button>
          </div>
          <p className="al-dim">
            Clicking Invite again with the same key replays the stored response instead of inviting
            twice. Try the same email in another case for a 409.
          </p>
        </section>

        <section className="al-card">
          <h3>Members ({members?.ok ? members.body.seats : "?"} seats, from a counter)</h3>
          {members && !members.ok ? <ResultView result={members} /> : null}
          <ul className="al-list">
            {members?.ok
              ? members.body.page.items.map((m) => (
                  <li key={m.memberId}>
                    <span style={{ flex: 1 }}>{m.email}</span>
                    <span className="al-dim">{m.role}</span>
                    <button
                      type="button"
                      onClick={() => run(call("DELETE", `${ORG}/members/${m.memberId}`))}
                    >
                      Remove
                    </button>
                  </li>
                ))
              : null}
          </ul>
        </section>

        <section className="al-card">
          <h3>Find by email (also an MCP tool)</h3>
          <div className="al-row">
            <input
              aria-label="Lookup email"
              value={lookup}
              onChange={(e) => setLookup(e.target.value)}
            />
            <button
              type="button"
              onClick={() =>
                run(call("GET", `${ORG}/members/by-email/${encodeURIComponent(lookup)}`))
              }
            >
              Find
            </button>
          </div>
        </section>

        <h3>Last response</h3>
        <ResultView result={last} />
      </main>
      <Inspector />
    </div>
  );
}
