"use client";

import { useState } from "react";

const SDK = "@aleph-lang/sdk";
const CLI = "@aleph-lang/cli";

const TABS: [string, string][] = [
  ["npm", `npm install ${SDK} && npm install -D ${CLI}`],
  ["pnpm", `pnpm add ${SDK} && pnpm add -D ${CLI}`],
  ["bun", `bun add ${SDK} && bun add -d ${CLI}`],
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
      <p className="install-note">
        Needs Node.js 24 or later. Aleph apps are ES modules, so set "type": "module" in your
        package.json.
      </p>
    </div>
  );
}
