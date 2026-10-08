#!/usr/bin/env bash
set -euo pipefail
top=$(git -C "$(dirname "$0")/../.." rev-parse --show-toplevel) || exit 2
cd "$top" || exit 2
export CARGO_NET_GIT_FETCH_WITH_CLI=true
if [ "${1:-}" = "--grow" ]; then
  export AGNOSTIC_GROW=1
fi
cargo test -q --test agnostic -- --ignored --exact rewrite_the_baseline
echo "tests/agnostic/baseline.txt rewritten from the tracked files"
