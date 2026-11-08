#!/usr/bin/env node
// Packs the public packages into tarballs ready for `npm publish`.
//
//   node scripts/release-pack.mjs --version 0.1.0 [--out release]
//
// Run `pnpm build` first. Each package is staged with its built files, a README, and the license,
// and its package.json gets the release version. Dependencies on other Aleph packages become that
// exact version, so @aleph-lang/sdk and @aleph-lang/cli always install matching versions of
// @aleph-lang/manifest and @aleph-lang/core. Publish the tarballs in the order listed in
// `release/publish-order.txt` (dependencies first).
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
    out: { type: "string", default: "release" },
  },
});
const version = values.version;
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error(`--version must be a semver version like 0.1.0, got ${version}`);
}
const out = resolve(values.out);

/** The packages to release, dependencies first: [directory, npm name, tarball name]. */
const PACKAGES = [
  ["packages/manifest", "@aleph-lang/manifest", "aleph-lang-manifest.tgz"],
  ["packages/core-wasm", "@aleph-lang/core", "aleph-lang-core.tgz"],
  ["packages/sdk", "@aleph-lang/sdk", "aleph-lang-sdk.tgz"],
  ["packages/cli", "@aleph-lang/cli", "aleph-lang-cli.tgz"],
];
const released = new Set(PACKAGES.map(([, name]) => name));

const INSTALL = [
  "```bash",
  "npm install @aleph-lang/sdk",
  "npm install --save-dev @aleph-lang/cli",
  "npm pkg set type=module   # Aleph apps are ES modules; new npm projects default to CommonJS",
  "```",
];

function readme(name, description) {
  return [
    `# ${name}`,
    "",
    description,
    "",
    "In a project on Node.js 24 or later:",
    "",
    ...INSTALL,
    "",
    "Then follow the [quickstart](https://aleph-lang.com/docs/quickstart/).",
    "",
    "Docs: https://aleph-lang.com/docs/ · Source: https://github.com/gubkin-labs/aleph",
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

  // Other Aleph packages are pinned to this exact version; dev dependencies aren't shipped.
  const dependencies = Object.fromEntries(
    Object.entries(pkg.dependencies ?? {}).map(([dep, range]) => {
      if (!range.startsWith("workspace:")) return [dep, range];
      if (!released.has(dep)) throw new Error(`${name} depends on ${dep}, which isn't released`);
      return [dep, version];
    }),
  );
  // The "@aleph/source" export condition points at src/, which isn't shipped.
  const exports = JSON.parse(
    JSON.stringify(pkg.exports, (key, value) => (key === "@aleph/source" ? undefined : value)),
  );
  const manifest = {
    ...pkg,
    version,
    exports,
    dependencies,
    devDependencies: undefined,
    scripts: undefined,
    engines: { node: ">=24" },
    homepage: "https://aleph-lang.com",
    repository: { type: "git", url: `git+https://github.com/${REPO}.git`, directory: dir },
    bugs: { url: `https://github.com/${REPO}/issues` },
    publishConfig: { access: "public" },
  };
  if (Object.keys(dependencies).length === 0) delete manifest.dependencies;
  writeFileSync(join(stage, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`);

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

writeFileSync(join(out, "publish-order.txt"), `${PACKAGES.map(([, , file]) => file).join("\n")}\n`);
writeFileSync(
  join(out, "RELEASE_NOTES.md"),
  [
    "## Install",
    "",
    "In a project on Node.js 24 or later:",
    "",
    ...INSTALL,
    "",
    "Then follow the [quickstart](https://aleph-lang.com/docs/quickstart/).",
    "",
    "## Packages",
    "",
    ...PACKAGES.map(
      ([, name]) =>
        `- [\`${name}@${version}\`](https://www.npmjs.com/package/${name}/v/${version})`,
    ),
    "",
  ].join("\n"),
);
console.log(`wrote ${join(values.out, "RELEASE_NOTES.md")}`);
