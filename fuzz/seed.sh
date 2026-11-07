#!/usr/bin/env bash
# Seeds the parse_bytes corpus with the golden and invalid conformance manifests.
# Usage: fuzz/seed.sh   (the structure-aware targets embed the golden manifests and need no seeds)
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p corpus/parse_bytes corpus/parse_mutated corpus/execute
for case in ../conformance/cases/*/; do
  cp "$case/manifest.json" "corpus/parse_bytes/golden-$(basename "$case").json"
done
for invalid in ../conformance/invalid/*.json; do
  node -e 'const fs = require("node:fs");
    fs.writeFileSync(process.argv[2], JSON.stringify(JSON.parse(fs.readFileSync(process.argv[1], "utf8")).manifest));' \
    "$invalid" "corpus/parse_bytes/invalid-$(basename "$invalid")"
done
echo "seeded $(ls corpus/parse_bytes | wc -l | tr -d ' ') parse_bytes inputs"
