"use client";

import { useState } from "react";

const TABS: [string, string][] = [
  ["npm", "npm install @aleph/sdk && npm install -D @aleph/cli"],
  ["pnpm", "pnpm add @aleph/sdk && pnpm add -D @aleph/cli"],
  ["bun", "bun add @aleph/sdk && bun add -d @aleph/cli"],
  [
    "from source",
    "git clone https://github.com/gubkin-labs/aleph.git && cd aleph && pnpm install && pnpm build",
  ],
];

export function Install() {
  const [tab, setTab] = useState(0);
  const [copied, setCopied] = useState(false);
  const command = TABS[tab]?.[1] ?? "";

  return (
    <div className="install">
      <div className="install-tabs" role="tablist">
        {TABS.map(([name], i) => (
          <button
            key={name}
            type="button"
            role="tab"
            aria-selected={i === tab}
            className={i === tab ? "active" : undefined}
            onClick={() => {
              setTab(i);
              setCopied(false);
            }}
          >
            {name}
          </button>
        ))}
      </div>
      <div className="install-body">
        <code>
          <span className="prompt">$ </span>
          {command}
        </code>
        <button
          type="button"
          className="install-copy"
          onClick={() => {
            navigator.clipboard?.writeText(command).then(() => setCopied(true));
          }}
        >
          [ {copied ? "copied" : "copy"} ]
        </button>
      </div>
      {tab < 3 && (
        <p className="install-note">
          TBD: the packages aren't on npm yet. Use “from source” for now.
        </p>
      )}
    </div>
  );
}
