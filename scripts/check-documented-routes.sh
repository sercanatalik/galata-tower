#!/usr/bin/env bash
#
# The README names every route the tower serves, and every variable it reads.
#
# Both halves drifted before this existed, within eight commits: /v1/status
# shipped and the README kept saying "three routes", and GALATA_BROKER was read
# by the binary and documented nowhere -- so somebody deploying this tower could
# not point it at a broker from the documentation.
#
# Everything else in the README is prose: judgement, reasoning, the argument for
# the wall. None of that has a source of truth and none of it is checked. What
# is checked is the part that does:
#
#   routes     against openapi.snapshot.json, which is generated from the
#              routes and already held current by check-contract-drift.sh
#   variables  against the binary's own std::env::var calls
#
# BOTH DIRECTIONS. A route present and undocumented is what happened; a
# documented route that no longer exists sends a reader to a 404.
#
# The variable check is TEXTUAL, in the manner of this family's other textual
# guards: it looks for `std::env::var("NAME")` and would miss a variable read
# through a helper. None is, and the pattern is named here so that the day one
# is introduced, the reason it slipped is readable rather than mysterious.
#
# NOT a documentation-drift tool. doc-drift, docsync and embedme solve the broad
# problem with AST analysis and a configuration file; this is two comparisons
# against two authorities the tree already keeps, and a dependency with a config
# would be more to maintain than the thing it replaces.
#
# Usage: check-documented-routes.sh [check|plant|plants|targets|expect] [root]

set -euo pipefail

VERB=check
if [[ $# -gt 0 ]]; then
    case "$1" in check|plant|plants|targets|expect) VERB="$1"; shift ;; esac
fi
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
PLANT="${GALATA_GUARD_PLANT:-undocumented-route}"
MAIN="$ROOT/crates/galata-tower/src/main.rs"

case "$VERB" in
    plants)
        echo undocumented-route
        echo undocumented-variable
        exit 0
        ;;
    targets)
        echo "README.md"
        echo "openapi.snapshot.json"
        echo "crates/galata-tower/src/main.rs"
        exit 0
        ;;
    expect)
        case "$PLANT" in
            undocumented-route) echo "/v1/planted" ;;
            undocumented-variable) echo "GALATA_PLANTED" ;;
        esac
        exit 0
        ;;
    plant)
        case "$PLANT" in
            # A route in the contract that the README does not name.
            undocumented-route)
                python3 - "$ROOT/openapi.snapshot.json" <<'PLANTPY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1])
doc = json.loads(path.read_text())
doc["paths"]["/v1/planted"] = {"get": {"responses": {"200": {"description": "planted"}}}}
path.write_text(json.dumps(doc, indent=2) + "\n")
PLANTPY
                ;;
            # A variable the binary reads and the README does not name.
            undocumented-variable)
                python3 - "$MAIN" <<'PLANTPY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
marker = "#[tokio::main]"
assert marker in text, "the plant's target moved -- the PLANT is wrong, not the guard"
addition = 'fn _planted() -> Option<String> { std::env::var("GALATA_PLANTED").ok() }\n\n'
path.write_text(text.replace(marker, addition + marker, 1))
PLANTPY
                ;;
            *)
                echo "check-documented-routes: unknown plant $PLANT" >&2
                exit 2
                ;;
        esac
        echo "planted ($PLANT)" >&2
        exit 0
        ;;
esac

problems=$(python3 - "$ROOT" <<'PY'
import json, pathlib, re, sys

root = pathlib.Path(sys.argv[1])
readme = (root / "README.md").read_text()
problems = []

served = set(json.loads((root / "openapi.snapshot.json").read_text())["paths"])
# A route is documented by appearing in the README at all -- the list is where
# it belongs, and requiring a particular shape of line would be a formatting
# rule rather than a documentation one.
#
# THE PARAMETER SEGMENT IS PART OF THE PATH. `/v1/[a-z-]+` truncated
# `/v1/tape/{kind}` to `/v1/tape`, so the first route with a parameter was
# reported both as undocumented AND as documented-but-not-served -- one defect
# read as two. Found by the first such route, which is late but is what the
# guard is for.
ROUTE = r"/v1/[a-z-]+(?:/\{[a-z_]+\})?"
documented = set(re.findall(ROUTE, readme))
for route in sorted(served - documented):
    problems.append(f"{route} is served and the README does not name it")
for route in sorted(documented - served):
    problems.append(f"{route} is in the README and the contract does not have it")

source = (root / "crates/galata-tower/src/main.rs").read_text()
read = set(re.findall(r'std::env::var\("([A-Z_]+)"\)', source))
named = set(re.findall(r"\b([A-Z][A-Z_]{3,})\b", readme))
for var in sorted(read - named):
    problems.append(f"{var} is read by the binary and the README does not name it")

print("\n".join(problems))
PY
)

if [[ -n "$problems" ]]; then
    echo "check-documented-routes: the README does not describe this binary:" >&2
    echo "$problems" | sed 's/^/    /' >&2
    exit 1
fi

echo "documented routes: ok. the README names every route the contract has, and every variable the binary reads"
