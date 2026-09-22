#!/usr/bin/env bash
#
# Everything, in one command.
#
# Structural guards FIRST: they answer in under a second, and a wrong tree
# should be rejected before anything spends a minute compiling it. Then format,
# lints and tests, then the guard self-test -- which is the slowest because it
# copies the tree once per plant.
#
# NO `set -e`, deliberately. Failures are counted, not fatal: a run that stops
# at the first problem tells you one thing when it could have told you four,
# and the second run costs another compile.
#
# Usage: check-all.sh [root]

set -uo pipefail

ROOT="$(cd "${1:-$(dirname "${BASH_SOURCE[0]}")/..}" && pwd)"
cd "$ROOT"
failed=0

# Discovered, never listed. A `scripts/check-*.sh` that exists is a guard that
# runs, so adding one cannot be forgotten in a list -- which would be the same
# class of defect the guards exist to catch.
for check in "$ROOT"/scripts/check-*.sh; do
    [[ -e "$check" ]] || continue
    case "$check" in */check-all.sh) continue ;; esac
    if ! "$check" check "$ROOT"; then
        failed=$((failed + 1))
    fi
done

run() {
    local label="$1"; shift
    local out
    if out=$("$@" 2>&1); then
        echo "$label: ok"
    else
        echo "$label: FAILED" >&2
        echo "$out" | tail -30 >&2
        failed=$((failed + 1))
    fi
}

run "format" cargo fmt --all --check
run "lints" cargo clippy --all-targets -- -D warnings
run "tests" cargo test --quiet -p galata-tower

# Last, and the point of the whole file: a green run above means nothing if the
# guards cannot fail.
if ! "$ROOT/scripts/test-guards.sh" "$ROOT"; then
    failed=$((failed + 1))
fi

if (( failed > 0 )); then
    printf '\n%d part(s) failed\n' "$failed" >&2
    exit 1
fi
printf '\nall green\n'
