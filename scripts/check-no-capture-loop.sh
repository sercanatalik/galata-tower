#!/usr/bin/env bash
#
# THE WALL. The tower links no capture loop.
#
# The tower watches the RECORD, not the worker: a heartbeat is a claim, a closed
# partition still holding 1,412 segments is a fact on disk. Everything it serves
# comes from the store's own listing, so it has no use for the machinery that
# fills the store.
#
# THE RULE IS NOT "NO ASYNC". It is "nothing that exists to talk to a venue".
# `tokio`, `hyper` and `hyper-util` are here and are axum's; a server needs a
# server. So is `rustls`, once `galata-broker` arrives, because NATS runs over
# TLS -- and TLS exists to talk to anything, which is the opposite of a venue
# transport. This script forbade it until 2026-09-22, for a reason its own
# header said was not the rule.
#
# So the rule is asserted as the rule. `galata-datawatch` is taken with
# `default-features = false`; with the feature on, `cargo tree -e features`
# prints the line `galata-datawatch feature "capture"`, and with it off that
# line is absent. Grepping for it IS the assertion rather than a proxy for it.
# Measured in galata-datawatch's design/measured.md: a tree without `capture`
# links 279 crates where one with it links 541.
#
# The crate list below is a SECOND NET, not the definition. The feature
# assertion cannot see a venue transport added directly to this crate's
# manifest -- that leaves `capture` off -- so the transports are named too.
#
# NOT cargo-deny, though `[bans]` is the standard answer and this family runs
# `cargo deny check` elsewhere: cargo-deny DOES NOT APPLY BANS TO PATH
# DEPENDENCIES, and every galata dependency here is one until galata-datawatch
# publishes. A ban that silently skips the crates it is aimed at is worse than
# no ban. Its `[graph] all-features` setting is the related trap: without it a
# feature-gated dependency is absent from the graph and a forbidden one passes.
#
# Usage: check-no-capture-loop.sh [check|plant|plants|targets] [root]

set -euo pipefail

VERB=check
if [[ $# -gt 0 ]]; then
    case "$1" in check|plant|plants|targets) VERB="$1"; shift ;; esac
fi
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
PLANT="${GALATA_GUARD_PLANT:-capture-feature}"

if [[ ! -f "$ROOT/Cargo.toml" ]]; then
    echo "$(basename "$0"): $ROOT is not a galata-tower workspace; refusing to scan nothing and call it ok" >&2
    exit 2
fi

# A venue's transport. NOT rustls: see the header.
FORBIDDEN='tokio-tungstenite|tungstenite|reqwest'

case "$VERB" in
    plants)
        echo capture-feature
        echo venue-transport
        ;;
    targets)
        echo "Cargo.toml"
        echo "crates/galata-tower/Cargo.toml"
        ;;
    plant)
        case "$PLANT" in
            # One clause in a manifest, 85 crates in the tree. The RULE is
            # planted, not a line: the dependency gained `features = [...]`
            # in 730923e and a plant matching the whole line stopped planting.
            capture-feature)
                python3 - "$ROOT/Cargo.toml" <<'PLANTPY'
import pathlib, re, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
line = re.compile(r'^(galata-datawatch\s*=\s*\{[^}\n]*?),\s*default-features\s*=\s*false', re.M)
planted, count = line.subn(r'\1', text, count=1)
assert count == 1, "no galata-datawatch line takes default-features = false -- the PLANT is wrong, not the guard"
path.write_text(planted)
PLANTPY
                ;;
            # A transport added straight to the crate, which leaves the feature
            # off -- the case the feature assertion cannot see, and the reason
            # the crate list is still here.
            venue-transport)
                python3 - "$ROOT/crates/galata-tower/Cargo.toml" <<'PLANTPY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
marker = "[dependencies]\n"
assert marker in text, "the plant's target moved -- the PLANT is wrong, not the guard"
at = text.index(marker) + len(marker)
path.write_text(text[:at] + 'reqwest = "0.13"  # planted by check-no-capture-loop.sh\n' + text[at:])
PLANTPY
                ;;
            *)
                echo "check-no-capture-loop: unknown plant $PLANT" >&2
                exit 2
                ;;
        esac
        echo "planted ($PLANT)" >&2
        ;;
    check)
        cd "$ROOT"
        # The rule itself.
        if cargo tree -p galata-tower -e features 2>/dev/null \
            | grep -qF 'galata-datawatch feature "capture"'; then
            echo "check-no-capture-loop: the capture feature is ON." >&2
            echo "  galata-datawatch must be taken with default-features = false. The tower" >&2
            echo "  watches the record; it has no use for the machinery that fills it." >&2
            exit 1
        fi
        # The second net: a transport added directly leaves the feature off.
        FOUND=$(cargo tree -p galata-tower --prefix none 2>/dev/null \
            | awk '{print $1}' | sort -u \
            | grep -xE "$FORBIDDEN" || true)
        if [[ -n "$FOUND" ]]; then
            echo "check-no-capture-loop: a venue transport is in the tree:" >&2
            echo "$FOUND" | sed 's/^/    /' >&2
            echo "  The capture feature is off, so this was added directly. A screen that" >&2
            echo "  reads parquet must not compile a websocket stack to do it." >&2
            exit 1
        fi
        TOTAL=$(cargo tree -p galata-tower --prefix none 2>/dev/null | awk '{print $1}' | sort -u | wc -l | tr -d ' ')
        echo "no capture loop: ok. the capture feature is off, and no venue transport in $TOTAL crates"
        ;;
esac
