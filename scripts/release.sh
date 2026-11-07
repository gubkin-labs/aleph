#!/usr/bin/env bash
# Publishes a release from this machine, with your own npm login:
#
#   pnpm release 0.1.0
#
# 1. Checks you're on a clean, pushed main, logged in to npm, and that the version is new.
# 2. Runs every check (`pnpm verify`), then packs the four packages (scripts/release-pack.mjs).
# 3. Publishes @aleph-lang/manifest, core, sdk, and cli to npm, dependencies first. npm may ask
#    you to confirm in the browser or enter a one-time code. A package already published at this
#    version is skipped, so you can run it again after a failure.
# 4. Tags the commit v<version>, pushes the tag, and creates the GitHub release with the notes.
#
# A prerelease version like 0.2.0-beta.1 goes to the npm tag "next", so `npm install` keeps getting
# the latest stable release. The Release workflow in GitHub Actions does the same from CI.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:-}"
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "usage: pnpm release <version>, like 0.1.0 (got '$VERSION')" >&2
  exit 2
fi
TAG=latest
PRERELEASE=()
if [[ "$VERSION" == *-* ]]; then
  TAG=next
  PRERELEASE=(--prerelease)
fi

step() { printf '\n\033[1m▶ %s\033[0m\n' "$1"; }
fail() {
  printf '\033[31m✖ %s\033[0m\n' "$1" >&2
  exit 1
}

step "Checking the release can go ahead"
[[ "$(git branch --show-current)" == "main" ]] || fail "switch to main first"
[[ -z "$(git status --porcelain)" ]] || fail "commit or stash your changes first"
git fetch -q origin main
[[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] ||
  fail "main isn't the same as origin/main: pull or push first"
if git rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null ||
  git ls-remote --exit-code --tags origin "refs/tags/v$VERSION" >/dev/null; then
  fail "tag v$VERSION already exists"
fi
NPM_USER=$(npm whoami 2>/dev/null) || fail "log in to npm first: npm login"
gh auth status >/dev/null 2>&1 || fail "log in to GitHub first: gh auth login"
echo "publishing v$VERSION as npm user $NPM_USER (npm tag: $TAG)"

step "Verifying"
scripts/ci.sh

step "Packing"
node scripts/release-pack.mjs --version "$VERSION" --out release

step "Publishing to npm"
while read -r file; do
  name=$(tar -xzOf "release/$file" package/package.json |
    node -p "JSON.parse(require('fs').readFileSync(0)).name")
  if npm view "$name@$VERSION" version >/dev/null 2>&1; then
    echo "$name@$VERSION is already on npm, skipping"
    continue
  fi
  npm publish "./release/$file" --access public --tag "$TAG"
done <release/publish-order.txt

step "Tagging and creating the GitHub release"
git tag -a "v$VERSION" -m "Aleph v$VERSION"
git push -q origin "v$VERSION"
gh release create "v$VERSION" \
  --verify-tag \
  --title "Aleph v$VERSION" \
  --notes-file release/RELEASE_NOTES.md \
  ${PRERELEASE[@]+"${PRERELEASE[@]}"}

printf '\n\033[1;32m✔ released v%s\033[0m\n' "$VERSION"
echo "https://www.npmjs.com/package/@aleph-lang/sdk/v/$VERSION"
