// Builds crates/aleph-core-wasm for wasm32 and generates Node bindings into ./wasm.
import { execFileSync } from "node:child_process";
import { existsSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const pkg = join(dirname(fileURLToPath(import.meta.url)), "..");
const root = join(pkg, "..", "..");
const run = (cmd, args) => execFileSync(cmd, args, { cwd: root, stdio: "inherit" });

run("cargo", [
  "build",
  "-p",
  "aleph-core-wasm",
  "--target",
  "wasm32-unknown-unknown",
  "--release",
  "--locked",
]);
const wasm = join(root, "target/wasm32-unknown-unknown/release/aleph_core_wasm.wasm");
if (!existsSync(wasm)) {
  throw new Error(`expected ${wasm} to exist after cargo build`);
}
run("wasm-bindgen", [
  wasm,
  "--out-dir",
  join(pkg, "wasm"),
  "--target",
  "nodejs",
  "--no-typescript",
]);
// wasm-bindgen's Node target emits CommonJS; mark the directory so Node doesn't treat it as ESM.
writeFileSync(join(pkg, "wasm", "package.json"), `${JSON.stringify({ type: "commonjs" })}\n`);
