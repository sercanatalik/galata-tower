#!/usr/bin/env bash
#
# The tower reads status. It does not read the firehose.
#
# `markets.` and `status.` are separate subject roots, and `Subject::status`
# says why in its own doc comment: so a dashboard watching every process can
# subscribe `status.>` WITHOUT also receiving market data. The separation is
# load-bearing only if somebody takes advantage of it, and until this guard
# landed the `reader` grant took both roots anyway.
#
# The predecessor decided this and wrote down the reason, which is not
# re-derived here: market data for a chart comes from the record, where an hour
# of candles already lives, because *an identity that could read every venue's
# firehose is exactly what a password on an operator's laptop should not be*.
#
# The server enforces the grant. This exists because the grant fails in the
# WRONG DIRECTION: somebody writes `markets.>`, the server refuses it, and the
# obvious repair is to widen the grant. A guard in the repository that would
# have written the subscription fails first, and says why.
#
# TEXTUAL, and that is a real limit. It greps for a literal market subject; a
# subject assembled at runtime from parts would pass it. The rule is that
# nobody writes one, so a hit is always a finding — the same bargain
# check-no-float-money.sh states and for the same reason.
#
# Its sibling one level down is check-no-capture-loop.sh: there the tower must
# not LINK a venue transport, here it must not ADDRESS the venues' data.
#
# Usage: check-no-market-reach.sh [check|plant|targets] [root]

set -euo pipefail

VERB=check
if [[ $# -gt 0 ]]; then
    case "$1" in check|plant|targets) VERB="$1"; shift ;; esac
fi
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
SRC="$ROOT/crates/galata-tower/src"

if [[ ! -d "$SRC" ]]; then
    echo "$(basename "$0"): $ROOT has no crates/galata-tower/src — refusing to scan nothing and call it ok" >&2
    exit 2
fi

PLANT="$SRC/main.rs"

case "$VERB" in
    targets)
        echo "crates/galata-tower/src/main.rs"
        exit 0
        ;;
    plant)
        # The ordinary mistake: the screen wants live prices, `markets.>` is
        # right there, and the grant is one line away from allowing it.
        printf '\n// planted by check-no-market-reach.sh\nconst _PLANTED: &str = "markets.>";\n' >>"$PLANT"
        echo "planted in $PLANT" >&2
        exit 0
        ;;
esac

# `markets.` anywhere in a string, and `Subject::market`, which builds one
# without the literal ever appearing. Comments are exempt: this file's own
# reasoning names the subject, and a guard that refuses the explanation of
# itself is one people route around.
hits=$(
    grep -rn --include='*.rs' -E '"markets\.|Subject::market\b' "$SRC" \
        | grep -vE '^\S+:[0-9]+:\s*(//|///|//!)' \
        || true
)

if [[ -n "$hits" ]]; then
    echo "no market reach: the tower is addressing market data, which comes from the record" >&2
    echo "$hits" >&2
    exit 1
fi

echo "no market reach: ok — the tower names no markets. subject (textual: a subject built from parts at runtime is outside this)"
