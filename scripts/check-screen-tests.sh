#!/usr/bin/env bash
#
# THE SCREEN'S ARITHMETIC IS EXECUTED, NOT ONLY COMPARED.
#
# The server has 33 tests. Until this landed the screen had none — not few,
# none: no runner, no test file, and nothing in the gate that ran a line of
# TypeScript. What was held was that the contract matched, that `dist` was the
# build of `src`, and that certain spellings did not appear. None of those run
# the code, and the code they did not run is where the reasoning lives:
#
#   money.ts        every price the screen shows, and the ONE file the money
#                   guard permits a float in — the one place in this tree
#                   where a wrong number would look right
#   live/status.ts  arithmetic across two machines' clocks, where which
#                   subtraction is meaningful was held by a comment
#   whySilent       four states that were one sentence, true of one of them
#
# **`environment: node`, deliberately.** No jsdom, no testing-library. Every
# function under test takes values and returns values; a DOM would cost setup
# on every run and hold nothing extra.
#
# WHAT IT CANNOT SEE: nothing here renders a component. A panel that computes
# correctly and displays nothing still passes — which is not hypothetical, it
# is exactly what happened when two panels asked for wall-clock windows
# against a thirty-three-hour-old tape and drew empty tables. The answer to
# that is to open the browser, and it stays the answer.
#
# Measured at about 130ms.
#
# Usage: check-screen-tests.sh [check|plant|targets] [root]

set -euo pipefail

VERB=check
if [[ $# -gt 0 ]]; then
    case "$1" in check|plant|targets) VERB="$1"; shift ;; esac
fi
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
UI="$ROOT/ui"

if [[ ! -d "$UI/src" ]]; then
    echo "$(basename "$0"): $ROOT has no ui/src — refusing to scan nothing and call it ok" >&2
    exit 2
fi

PLANT="$UI/src/contract/money.ts"

case "$VERB" in
    targets)
        echo "ui/src/contract/money.ts"
        exit 0
        ;;
    plant)
        # **Break the subject, not the test.** A parse that swallows what it
        # could not read is the failure `dec` throwing exists to prevent, and
        # it is the change somebody would plausibly make to "stop the panel
        # crashing" — which is why it is the one planted.
        python3 - "$PLANT" <<'PLANTPY'
import sys, pathlib
path = pathlib.Path(sys.argv[1])
text = path.read_text()
marker = "  return new Decimal(s)\n"
assert marker in text, "the plant's target moved — the PLANT is wrong, not the guard"
swallow = "  try {\n    return new Decimal(s)\n  } catch {\n    return null\n  }\n"
path.write_text(text.replace(marker, swallow, 1))
PLANTPY
        echo "planted in $PLANT" >&2
        exit 0
        ;;
esac

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
    echo "screen tests: the screen's dependencies would not install" >&2
    echo "$installed" | tail -20 >&2
    exit 1
fi

if ! output=$(pnpm test 2>&1); then
    echo "screen tests: the screen's arithmetic does not hold" >&2
    echo "$output" | grep -E "FAIL|AssertionError|Error:|✕|×|Tests " | head -12 >&2
    exit 1
fi

passed=$(echo "$output" | grep -oE "Tests +[0-9]+ passed" | grep -oE "[0-9]+" | head -1)
echo "screen tests: ok. ${passed:-?} assertion(s) over the screen's money, clocks and classification (no DOM: a panel that computes right and draws nothing still passes — open the browser)"
