#!/usr/bin/env node
// Packs the public packages into self-contained tarballs for a GitHub release.
//
//   node scripts/release-pack.mjs --version 0.1.0-alpha.1 [--base-url URL] [--out release]
//
// Run `pnpm build` first. Each package is staged with its built files, a README, and the license,
// and its package.json gets the release version. Dependencies on other Aleph packages point at
// their tarballs in the same release (`--base-url`), so installing @aleph/sdk and @aleph/cli by
// URL pulls in matching versions of @aleph/manifest and @aleph/core. Tarballs have stable names
// (aleph-sdk.tgz) so `releases/latest/download/aleph-sdk.tgz` always means the newest release.
import { execFileSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const REPO = "gubkin-labs/aleph";
const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const { values } = parseArgs({
  options: {
    version: { type: "string" },
    "base-url": { type: "string" },
    out: { type: "string", default: "release" },
  },
});
const version = values.version;
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error(`--version must be a semver version like 0.1.0-alpha.1, got ${version}`);
}
const baseUrl = (
  values["base-url"] ?? `https://github.com/${REPO}/releases/download/v${version}`
).replace(/\/$/, "");
const out = resolve(values.out);

/** The packages to release, dependencies first: [directory, npm name, tarball name]. */
const PACKAGES = [
  ["packages/manifest", "@aleph/manifest", "aleph-manifest.tgz"],
  ["packages/core-wasm", "@aleph/core", "aleph-core.tgz"],
  ["packages/sdk", "@aleph/sdk", "aleph-sdk.tgz"],
  ["packages/cli", "@aleph/cli", "aleph-cli.tgz"],
];
const tarballOf = new Map(PACKAGES.map(([, name, file]) => [name, `${baseUrl}/${file}`]));

const ALPHA = [
  "> [!WARNING]",
  "> **Alpha.** Aleph is an early alpha and isn't ready for production. The language, the manifest",
  "> format, and these packages will change without notice between releases.",
].join("\n");

const latest = (file) => `https://github.com/${REPO}/releases/latest/download/${file}`;
const install = (url) => [
  "```bash",
  `npm install ${url("aleph-sdk.tgz")}`,
  `npm install -D ${url("aleph-cli.tgz")}`,
  "```",
];

function readme(name, description) {
  return [
    `# ${name}`,
    "",
    description,
    "",
    ALPHA,
    "",
    "Aleph isn't published to npm yet. Install it from the latest GitHub release:",
    "",
    ...install(latest),
    "",
    "Docs: https://aleph-lang.vercel.app/docs/ · Source: https://github.com/gubkin-labs/aleph",
    "",
  ].join("\n");
}

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

for (const [dir, name, file] of PACKAGES) {
  const pkgDir = join(root, dir);
  const pkg = JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf8"));
  if (pkg.name !== name) throw new Error(`${dir} is ${pkg.name}, expected ${name}`);

  const stage = join(out, "stage", file.replace(/\.tgz$/, ""));
  mkdirSync(stage, { recursive: true });
  for (const entry of pkg.files ?? []) {
    const from = join(pkgDir, entry);
    if (!existsSync(from)) throw new Error(`${dir}/${entry} is missing: run \`pnpm build\` first`);
    cpSync(from, join(stage, entry), { recursive: true });
  }
  cpSync(join(root, "LICENSE"), join(stage, "LICENSE"));
  writeFileSync(join(stage, "README.md"), readme(name, pkg.description));

  // Other Aleph packages come from this release's tarballs; dev dependencies aren't shipped.
  const dependencies = Object.fromEntries(
    Object.entries(pkg.dependencies ?? {}).map(([dep, range]) => {
      if (!range.startsWith("workspace:")) return [dep, range];
      const url = tarballOf.get(dep);
      if (!url) throw new Error(`${name} depends on ${dep}, which isn't released`);
      return [dep, url];
    }),
  );
  // The "@aleph/source" export condition points at src/, which isn't shipped.
  const exports = JSON.parse(
    JSON.stringify(pkg.exports, (key, value) => (key === "@aleph/source" ? undefined : value)),
  );
  const released = {
    ...pkg,
    version,
    exports,
    dependencies,
    devDependencies: undefined,
    scripts: undefined,
    engines: { node: ">=24" },
    homepage: "https://aleph-lang.vercel.app",
    repository: { type: "git", url: `git+https://github.com/${REPO}.git`, directory: dir },
    bugs: { url: `https://github.com/${REPO}/issues` },
  };
  if (Object.keys(dependencies).length === 0) delete released.dependencies;
  writeFileSync(join(stage, "package.json"), `${JSON.stringify(released, null, 2)}\n`);

  const packed = execFileSync("npm", ["pack", "--silent", "--pack-destination", out], {
    cwd: stage,
    encoding: "utf8",
  })
    .trim()
    .split("\n")
    .at(-1);
  renameSync(join(out, packed), join(out, file));
  console.log(`packed ${name}@${version} → ${join(values.out, file)}`);
}
rmSync(join(out, "stage"), { recursive: true, force: true });

const pinned = (f) => `${baseUrl}/${f}`;
writeFileSync(
  join(out, "RELEASE_NOTES.md"),
  [
    ALPHA,
    "",
    "## Install",
    "",
    "In a project with Node.js 24 or later:",
    "",
    ...install(pinned),
    "",
    "Or always take the newest release:",
    "",
    ...install(latest),
    "",
    "Then follow the [quickstart](https://aleph-lang.vercel.app/docs/quickstart/).",
    "",
    "## Packages",
    "",
    ...PACKAGES.map(([, name, f]) => `- \`${name}\` ${version}: \`${f}\``),
    "",
  ].join("\n"),
);
console.log(`wrote ${join(values.out, "RELEASE_NOTES.md")}`);
