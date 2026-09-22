#!/usr/bin/env bash
#
# Money never touches a float.
#
# Every decimal the contract carries is a string, and the browser parses it
# with decimal.js. A `parseFloat(` or `Number(` on a money path rounds a
# position and looks right — the kind of wrong nobody notices until the
# figure disagrees with the venue's — so both are refused anywhere under src/
# except the one file that converts a Decimal to a plotting number and says
# why: src/contract/money.ts.
#
# Textual, like galata's own guards: it cannot tell a money `Number(` from a
# harmless one, and does not try. The rule is that the harmless ones are not
# written either, so a hit is always a finding.
#
# Carried from the predecessor almost verbatim, including the limitation above:
# the reasoning is the part that was paid for. What changed is where the sources
# live -- `ui/src` here, because this repository holds both halves, where the
# predecessor was the screen alone.
#
# Usage: check-no-float-money.sh [check|plant|targets] [root]

set -euo pipefail

VERB=check
if [[ $# -gt 0 ]]; then
    case "$1" in check|plant|targets) VERB="$1"; shift ;; esac
fi
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
SRC="$ROOT/ui/src"

if [[ ! -d "$SRC" ]]; then
    echo "$(basename "$0"): $ROOT has no ui/src — refusing to scan nothing and call it ok" >&2
    exit 2
fi

PLANT="$SRC/contract/client.ts"

case "$VERB" in
    targets)
        echo "ui/src/contract/money.ts"
        echo "ui/src/contract/client.ts"
        exit 0
        ;;
    plant)
        # A float made outside the one file that may. The ordinary mistake, and
        # it looks right: a rounded position reads fine until it disagrees with
        # the venue's.
        printf '\n// planted by check-no-float-money.sh\nexport const _planted = Number("1.23")\n' >>"$PLANT"
        echo "planted in $PLANT" >&2
        exit 0
        ;;
esac

hits=$(
    grep -rn --include='*.ts' --include='*.tsx' -E '\bparseFloat\(|\bNumber\(' "$SRC" \
        | grep -v '/src/contract/money.ts:' \
        | grep -vE '^\S+:[0-9]+:\s*//' \
        || true
)

if [[ -n "$hits" ]]; then
    echo "no float money: a decimal is being made into a float outside ui/src/contract/money.ts" >&2
    echo "$hits" >&2
    exit 1
fi

echo "no float money: ok — parseFloat and Number( appear only in ui/src/contract/money.ts"
