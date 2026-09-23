#!/usr/bin/env bash
#
# The committed contract is what the code serves.
#
# The predecessor kept 669 lines of hand-typed TypeScript, a 233-line client and
# a 343-line fixture test, and that test's own header says what it could not do:
# "a field the server REMOVES fails naming it, and a field it ADDS passes. One
# direction." Its fixtures were produced by a `cargo run` in a DIFFERENT
# repository and consumed here, reconciled by somebody remembering.
#
# So: the binary that serves the routes prints the document, the document is
# committed, and this diffs the bytes. Both directions, no distance, and a
# reviewer reading a pull request sees the contract change as a diff.
#
# A byte comparison is only fair over a deterministic generator, and utoipa's
# is one -- proved by generating twice and comparing, below.
#
#   check-contract-drift.sh            fail if the snapshot or the types are stale
#   check-contract-drift.sh --write    refresh both, in one command
#
# `--write` is here rather than in a second script because a check that tells
# you it failed without telling you how to fix it is a check people route
# around.
#
# Usage: check-contract-drift.sh [check|plant|targets|--write] [root]

set -euo pipefail

VERB=check
if [[ $# -gt 0 ]]; then
    case "$1" in check|plant|targets) VERB="$1"; shift ;; --write) VERB=write; shift ;; esac
fi
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

SNAPSHOT="$ROOT/openapi.snapshot.json"
TYPES="$ROOT/ui/src/contract/api.d.ts"

case "$VERB" in
    targets)
        echo "openapi.snapshot.json"
        echo "crates/galata-tower/src/main.rs"
        ;;
    plant)
        # The contract changed and the snapshot was not refreshed.
        #
        # It edits a RESPONSE DESCRIPTION rather than adding a field, and that
        # is deliberate: adding a field to a struct stops the crate compiling
        # until its initializer is updated too, so the guard would fail with a
        # build error and prove nothing about drift. A plant must fail the
        # guard for the guard's own reason.
        python3 - "$ROOT/crates/galata-tower/src/main.rs" <<'PLANTPY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
marker = 'description = "The archive root and the tape\'s prune columns"'
assert marker in text, "the plant's target moved -- the PLANT is wrong, not the guard"
path.write_text(text.replace(marker, 'description = "planted by check-contract-drift.sh"', 1))
PLANTPY
        echo "planted in crates/galata-tower/src/main.rs" >&2
        ;;
    check|write)
        cd "$ROOT"
        cargo build --quiet -p galata-tower
        # **Where cargo actually put it**, not `./target`. This said
        # `./target/debug/galata-tower`, which is right only when nobody has
        # set `CARGO_TARGET_DIR` — a common setting for a shared build cache,
        # and one the guard harness now uses so that eighteen copies of this
        # tree do not each compile 245 crates. With it set, the guard failed
        # with "No such file or directory" and the harness correctly reported
        # it as a guard that cannot be said to catch anything.
        BIN="${CARGO_TARGET_DIR:-$ROOT/target}/debug/galata-tower"
        FRESH="$(mktemp)"; trap 'rm -f "$FRESH"' EXIT
        "$BIN" --dump-openapi >"$FRESH"

        # The determinism the byte comparison rests on, checked rather than
        # assumed: a flaky generator would make this guard fail at random and
        # teach people to ignore it.
        SECOND="$(mktemp)"
        "$BIN" --dump-openapi >"$SECOND"
        if ! cmp -s "$FRESH" "$SECOND"; then
            rm -f "$SECOND"
            echo "check-contract-drift: the generator is not deterministic; two runs differ." >&2
            echo "  A byte comparison cannot hold over a generator that does not repeat." >&2
            exit 1
        fi
        rm -f "$SECOND"

        if [[ "$VERB" == write ]]; then
            cp "$FRESH" "$SNAPSHOT"
            mkdir -p "$(dirname "$TYPES")"
            (cd "$ROOT/ui" && npx --yes openapi-typescript ../openapi.snapshot.json -o src/contract/api.d.ts)
            echo "contract: written. $SNAPSHOT and $TYPES refreshed"
            exit 0
        fi

        if [[ ! -f "$SNAPSHOT" ]]; then
            echo "check-contract-drift: openapi.snapshot.json is missing. Run --write." >&2
            exit 1
        fi
        if ! diff -u "$SNAPSHOT" "$FRESH" >/tmp/contract-drift.diff; then
            echo "check-contract-drift: the committed contract is not what the code serves:" >&2
            head -30 /tmp/contract-drift.diff | sed 's/^/    /' >&2
            echo "  The code is the authority. Run: scripts/check-contract-drift.sh --write" >&2
            exit 1
        fi
        # A missing generated file fails rather than passing because there was
        # nothing to compare.
        if [[ ! -f "$TYPES" ]]; then
            echo "check-contract-drift: $TYPES is missing, so the screen has no types." >&2
            echo "  Run: scripts/check-contract-drift.sh --write" >&2
            exit 1
        fi
        echo "contract: ok. the committed document is what the code serves, and the screen's types come from it"
        ;;
esac
