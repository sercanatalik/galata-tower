#!/usr/bin/env bash
#
# The built screen is the build of the sources.
#
# `ui/dist` is committed because rust-embed reads it at compile time, and
# nothing checked that it came from `ui/src`. So the ordinary mistake was
# invisible: edit the screen, forget `pnpm build`, and the binary serves the
# previous screen while every guard stays green and the diff looks complete.
# The same defect the contract snapshot has, which is why that has a check.
#
# MEASURED BEFORE THIS WAS WRITTEN, because a byte comparison over a generator
# that does not repeat is a flaky test rather than a check. Three consecutive
# builds produced an identical tree hash, with stable content-hashed filenames
# and nothing machine-specific inside -- no absolute path, no timestamp, no
# NODE_ENV.
#
# THE CLAIM IS NARROW AND IT IS STATED. Vite is NOT deterministic in general:
# upstream reports differing chunking between identical runs, and toolchain
# drift changes output hashes even with a correct lockfile. What holds is: with
# this lockfile, frozen, and this toolchain, the build repeats. If that stops
# being true the guard goes red, and that is worth learning -- see the message,
# which distinguishes a stale dist from a build that no longer repeats.
#
#   check-dist-drift.sh            fail if ui/dist is not the build of ui/src
#   check-dist-drift.sh --write    rebuild it
#
# Skips visibly without Node. GALATA_REQUIRE_DIST_CHECK=1 makes the skip a
# failure: a silent skip is worse than no check, because the gate then claims
# more than it did.
#
# Usage: check-dist-drift.sh [check|plant|targets|--write] [root]

set -euo pipefail

VERB=check
if [[ $# -gt 0 ]]; then
    case "$1" in check|plant|plants|targets|expect) VERB="$1"; shift ;; --write) VERB=write; shift ;; esac
fi
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
UI="$ROOT/ui"

case "$VERB" in
    targets)
        echo "ui/package.json"
        echo "ui/pnpm-lock.yaml"
        echo "ui/dist/index.html"
        exit 0
        ;;
    plants)
        echo stale-dist
        exit 0
        ;;
    expect)
        # The failure must name the file that differs, not merely be a failure:
        # a guard whose script broke also exits non-zero.
        echo "ui/dist is not the build of ui/src"
        exit 0
        ;;
    plant)
        # A source edit without a rebuild. The mistake this exists for.
        printf '\n/* planted by check-dist-drift.sh */\n.planted { color: red; }\n' >>"$UI/src/index.css"
        echo "planted in ui/src/index.css" >&2
        exit 0
        ;;
esac

if ! command -v pnpm >/dev/null 2>&1 || ! command -v node >/dev/null 2>&1; then
    if [[ -n "${GALATA_REQUIRE_DIST_CHECK:-}" ]]; then
        echo "check-dist-drift: node and pnpm are required and one is missing." >&2
        exit 1
    fi
    echo "dist drift: SKIPPED — node or pnpm is not installed, so ui/dist was not rebuilt."
    echo "  The committed screen is therefore UNVERIFIED in this run. Set"
    echo "  GALATA_REQUIRE_DIST_CHECK=1 to make this a failure instead."
    exit 0
fi

cd "$UI"
# **Not silenced.** This used to be `>/dev/null 2>&1`, and with `set -e` a
# failed install killed the guard with NO OUTPUT AT ALL — which is exactly
# how it behaved on its first CI run: no ok line, no failure line, just a
# missing guard and a count that said something had failed. A check that
# cannot say why it died is worse than no check.
#
# Frozen, because package.json carries `^` ranges: a plain install may resolve
# a newer minor and the result would be about the registry rather than the
# code. It is also what makes this runnable in a scratch copy, where there is
# no node_modules at all.
if ! installed=$(pnpm install --frozen-lockfile 2>&1); then
    echo "dist drift: the screen's dependencies would not install" >&2
    echo "$installed" | tail -20 >&2
    exit 1
fi

if [[ "$VERB" == write ]]; then
    pnpm build >/dev/null 2>&1
    echo "dist: written. ui/dist rebuilt from ui/src"
    exit 0
fi

# Into scratch, never over the committed tree: a check that overwrites what it
# checks cannot fail twice, and a half-run gate would leave the tree modified.
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/galata-dist-XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
pnpm exec vite build --outDir "$SCRATCH/dist" --emptyOutDir >/dev/null 2>&1

# The file SET as well as the bytes: content-hashed names mean a changed chunk
# arrives as a NEW PATH, so comparing only shared paths would pass while the
# trees differed.
if ! diff -rq "$UI/dist" "$SCRATCH/dist" >/tmp/dist-drift.diff 2>&1; then
    echo "check-dist-drift: ui/dist is not the build of ui/src:" >&2
    sed 's/^/    /' /tmp/dist-drift.diff >&2
    echo "  Either the screen was edited without rebuilding -- run:" >&2
    echo "      scripts/check-dist-drift.sh --write" >&2
    echo "  or the build no longer repeats, which is a fact about the toolchain" >&2
    echo "  rather than about the screen, and worth knowing on its own." >&2
    exit 1
fi

echo "dist drift: ok. the committed screen is the build of ui/src"
