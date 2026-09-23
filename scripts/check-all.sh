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

# **Timing, because deciding what to make faster needed hand-instrumenting
# twice this week.** Once a guard was spending most of its run purging caches
# whether or not anything had moved; once a sibling repository's wedged build
# was making every number a lie. Neither was visible from output that says what
# passed and nothing about what it cost.
#
# `SECONDS`, the shell's own counter, not `EPOCHREALTIME`: the bash on a
# developer's macOS is 3.2.57, where that and `EPOCHSECONDS` do not exist —
# they arrived in bash 5. CI has bash 5, so a script written against them would
# work in CI and print nothing where the work is done. One-second resolution,
# which is the resolution the question has: a guard reporting `0s` because it
# carried a cached verdict is exactly what is worth seeing.
#
# Two parallel arrays, because bash 3.2 has no `declare -A`.
TIMED_NAMES=()
TIMED_SECS=()

timed() {
    local label="$1"; shift
    local began=$SECONDS
    "$@"
    local outcome=$?
    TIMED_NAMES+=("$label")
    TIMED_SECS+=("$(( SECONDS - began ))")
    return $outcome
}

report_timings() {
    (( ${#TIMED_NAMES[@]} == 0 )) && return 0
    printf '\n=== where the time went (whole seconds; slowest first)\n'
    local i
    for (( i = 0; i < ${#TIMED_NAMES[@]}; i++ )); do
        printf '%6s  %s\n' "${TIMED_SECS[$i]}s" "${TIMED_NAMES[$i]}"
    done | sort -rn
    printf '%6s  %s\n' "${SECONDS}s" "the whole gate"
}
# On EXIT, so an interrupted run still says what it had measured.
trap report_timings EXIT

# Discovered, never listed. A `scripts/check-*.sh` that exists is a guard that
# runs, so adding one cannot be forgotten in a list -- which would be the same
# class of defect the guards exist to catch.
for check in "$ROOT"/scripts/check-*.sh; do
    [[ -e "$check" ]] || continue
    case "$check" in */check-all.sh) continue ;; esac
    # Each guard timed on its own: one number for "guards" would hide the one
    # worth looking at behind the several that are not.
    if ! timed "guard $(basename "$check" .sh)" "$check" check "$ROOT"; then
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

timed "format" run "format" cargo fmt --all --check
timed "lints" run "lints" cargo clippy --all-targets -- -D warnings
timed "tests" run "tests" cargo test --quiet -p galata-tower

# Last, and the point of the whole file: a green run above means nothing if the
# guards cannot fail.
if ! timed "the guards can fail" "$ROOT/scripts/test-guards.sh" "$ROOT"; then
    failed=$((failed + 1))
fi

if (( failed > 0 )); then
    printf '\n%d part(s) failed\n' "$failed" >&2
    exit 1
fi
printf '\nall green\n'
