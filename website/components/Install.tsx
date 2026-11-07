"use client";

import { useState } from "react";

// Alpha releases are tarballs on GitHub, not npm packages yet. Install a versioned URL: a lockfile
// records the file's checksum, and the "latest" link serves a new file on every release.
// `scripts/set-release-version.mjs` updates this on each release.
const VERSION = "0.1.0-alpha.2";
const RELEASE = `https://github.com/gubkin-labs/aleph/releases/download/v${VERSION}`;
const SDK = `${RELEASE}/aleph-sdk.tgz`;
const CLI = `${RELEASE}/aleph-cli.tgz`;

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
        Alpha: not ready for production, and it will change between releases. Installs v{VERSION}
        from GitHub.
      </p>
    </div>
  );
}
