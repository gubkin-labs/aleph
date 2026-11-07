#!/usr/bin/env node
// Points the install instructions at a release:
//
//   node scripts/set-release-version.mjs 0.1.0-alpha.2
//
// The README, the quickstart, and the homepage install box use versioned release URLs, because a
// lockfile records each tarball's checksum and the "latest" link serves a new file on every
// release. The release workflow runs this after publishing a release.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error(`usage: set-release-version.mjs <version>, like 0.1.0-alpha.2 (got ${version})`);
}
const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Each file, and the pattern holding its pinned version. */
const FILES = [
  ["README.md", /(releases\/download\/v)[0-9A-Za-z.-]+(\/aleph-)/g],
  ["website/content/docs/quickstart.mdx", /(releases\/download\/v)[0-9A-Za-z.-]+(\/aleph-)/g],
  ["website/components/Install.tsx", /(const VERSION = ")[0-9A-Za-z.-]+(";)/g],
];

for (const [file, pattern] of FILES) {
  const path = join(root, file);
  const before = readFileSync(path, "utf8");
  const matches = before.match(pattern)?.length ?? 0;
  if (matches === 0) throw new Error(`${file} has no pinned release version to update`);
  writeFileSync(path, before.replace(pattern, `$1${version}$2`));
  console.log(`${file}: ${matches} pinned to v${version}`);
}
