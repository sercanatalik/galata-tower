#!/usr/bin/env bash
#
# THE WALL. The tower links no capture loop.
#
# The tower watches the RECORD, not the worker: a heartbeat is a claim, a closed
# partition still holding 1,412 segments is a fact on disk. Everything it serves
# comes from the store's own listing, so it has no use for the machinery that
# fills the store -- and acquiring it would put a websocket stack and an HTTP
# client behind a screen that reads parquet.
#
# `galata-datawatch` is taken with `default-features = false` for exactly this.
# Measured in that repository's design/measured.md: a tree built without
# `capture` links 279 crates where one with it links 541.
#
# WHAT IS NOT FORBIDDEN, and the distinction is the whole point: `tokio`, `hyper`
# and `hyper-util` are present and legitimate -- they are axum's, and a server
# needs a server. The rule is not "no async"; it is "nothing that exists to talk
# to a venue".
#
# Asked of cargo rather than of the manifest, because a manifest is what
# somebody edits and a resolved tree is what runs.
#
# Usage: check-no-capture-loop.sh [check|plant|targets] [root]

set -euo pipefail

VERB=check
if [[ $# -gt 0 ]]; then
    case "$1" in check|plant|targets) VERB="$1"; shift ;; esac
fi
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

if [[ ! -f "$ROOT/Cargo.toml" ]]; then
    echo "$(basename "$0"): $ROOT is not a galata-tower workspace; refusing to scan nothing and call it ok" >&2
    exit 2
fi

# Every one of these enters only through galata-datawatch's `capture` feature.
FORBIDDEN='tokio-tungstenite|tungstenite|reqwest|rustls|rustls-pki-types|webpki-roots'

case "$VERB" in
    targets)
        echo "Cargo.toml"
        echo "crates/galata-tower/Cargo.toml"
        ;;
    plant)
        # The ordinary mistake: turning the capture feature back on, which is
        # one word in a manifest and 262 crates in the tree.
        python3 - "$ROOT/Cargo.toml" <<'PLANTPY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
marker = 'galata-datawatch = { path = "../galata-datawatch/crates/galata-datawatch", default-features = false }'
assert marker in text, "the plant's target moved -- the PLANT is wrong, not the guard"
replacement = 'galata-datawatch = { path = "../galata-datawatch/crates/galata-datawatch" }'
path.write_text(text.replace(marker, replacement, 1))
PLANTPY
        echo "planted in $ROOT/Cargo.toml" >&2
        ;;
    check)
        cd "$ROOT"
        FOUND=$(cargo tree -p galata-tower --prefix none 2>/dev/null \
            | awk '{print $1}' | sort -u \
            | grep -xE "$FORBIDDEN" || true)
        if [[ -n "$FOUND" ]]; then
            echo "check-no-capture-loop: the tower links the capture transport:" >&2
            echo "$FOUND" | sed 's/^/    /' >&2
            echo "  A screen that reads parquet must not compile a websocket stack to do it." >&2
            echo "  galata-datawatch is taken with default-features = false." >&2
            exit 1
        fi
        TOTAL=$(cargo tree -p galata-tower --prefix none 2>/dev/null | awk '{print $1}' | sort -u | wc -l | tr -d ' ')
        echo "no capture loop: ok. $TOTAL crates, and none of the venue transport among them"
        ;;
esac
