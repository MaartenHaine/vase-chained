#!/usr/bin/env bash
# The Linux Tiler's static checks (docs/linux-tiler/devtest-security.md §4.4): the metadata lint, the tripwires and
# eslint. CI runs this on the runner; `tools/linux/podman.sh lint NODE_DIR` runs it in the build image. It needs bash,
# python3, grep -P, and node and npm on PATH; npm installs only what tools/linux/eslint/package-lock.json pins, with
# install scripts off, into a temporary directory.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"

python3 -I tools/linux/lint-metadata.py --self-test
python3 -I tools/linux/lint-metadata.py vase-gnome/extension
bash tools/linux/tripwires.sh --self-test
bash tools/linux/tripwires.sh vase-gnome/extension

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp tools/linux/eslint/package.json tools/linux/eslint/package-lock.json tools/linux/eslint/eslint.config.js "$work/"
(cd "$work" && npm ci --ignore-scripts --no-audit --no-fund --no-update-notifier --loglevel=error)
# With --config, eslint takes the working directory (the repo) as its base path.
"$work/node_modules/.bin/eslint" --config "$work/eslint.config.js" vase-gnome
echo 'lint: eslint clean'
