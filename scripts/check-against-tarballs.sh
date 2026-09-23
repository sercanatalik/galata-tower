#!/usr/bin/env bash
#
# THE TOWER BUILDS AGAINST THE CRATES THAT WOULD SHIP, NOT ONLY THE CHECKOUT.
#
# `Cargo.toml` takes galata-datawatch and its siblings BY PATH, and says why:
# they publish nothing yet, and they "become registry dependencies when its
# Tier 10 lands, and the sibling checkout stops being load-bearing". Nothing
# checked that the second half is true.
#
# One repository over, `check-tarball-builds.sh` proves each crate compiles
# from its own tarball, including with `--no-default-features` — which is what
# this tower takes. That is the PRODUCER's side. This is the CONSUMER's: the
# tower, built exactly as a stranger would build it, against the packaged
# crates as registry dependencies rather than as paths.
#
# The difference is not hypothetical. A path dependency reaches every file on
# disk and resolves through the workspace; a registry dependency reaches only
# what `include` shipped and resolves through a rewritten manifest. A crate
# that is `publish = false` is invisible here and perfectly usable there.
#
# **The sibling is REPACKAGED first, and that is not belt-and-braces.**
# `target/package/<crate>-<version>/` is a staging directory that survives
# runs, and the guard harness plants violations into source before packaging —
# so a stale staging directory can hold a PLANTED copy of a crate. Observed
# 2026-09-23: this check failed against `const _PLANTED: () =
# this_does_not_exist();` left behind in the staged copy while the real source
# was clean.
#
# Usage: check-against-tarballs.sh [check|plant] [root]

set -euo pipefail

VERB=check
if [[ $# -gt 0 ]]; then
    case "$1" in check|plant) VERB="$1"; shift ;; esac
fi
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
SIBLING="$(cd "$ROOT/.." && pwd)/galata-datawatch"

if [[ "$VERB" == plant ]]; then
    # **Depend on the crate that does not publish.** `galata-datawatch-vault`
    # is `publish = false` on purpose, so it builds perfectly from the
    # checkout and does not exist for anybody else — which is precisely the
    # class of mistake this guard is for.
    # **Depend on the crate that does not publish.** `galata-datawatch-vault`
    # is `publish = false` on purpose, so it builds perfectly from the
    # checkout and does not exist for anybody else — precisely the class of
    # mistake this guard is for.
    #
    # BOTH halves, because the first draft planted only the workspace entry
    # and the guard stayed green: cargo never resolves a `[workspace
    # .dependencies]` line that no crate takes. An inert plant reports the
    # GUARD as broken when the PLANT is.
    python3 - "$ROOT/Cargo.toml" "$ROOT/crates/galata-tower/Cargo.toml" <<'PLANTPY'
import sys, pathlib
ws, crate = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])

text = ws.read_text()
marker = 'galata-wire = { path = "../galata-datawatch/crates/galata-wire" }\n'
assert marker in text, "the plant's target moved — the PLANT is wrong, not the guard"
ws.write_text(text.replace(
    marker,
    marker + 'galata-datawatch-vault = { path = "../galata-datawatch/crates/galata-datawatch-vault" }\n',
    1))

text = crate.read_text()
marker = "galata-datawatch.workspace = true\n"
assert marker in text, "the plant's second target moved — the PLANT is wrong, not the guard"
crate.write_text(text.replace(marker, marker + "galata-datawatch-vault.workspace = true\n", 1))
PLANTPY
    echo "planted in $ROOT/Cargo.toml" >&2
    exit 0
fi

if [[ ! -d "$SIBLING" ]]; then
    echo "against tarballs: no galata-datawatch checkout beside this one — refusing to skip and call it ok" >&2
    exit 2
fi

# Fresh tarballs, for the staging reason in the header.
if ! packaged=$(cd "$SIBLING" && cargo package --workspace --allow-dirty 2>&1); then
    echo "against tarballs: the sibling would not package, so there is nothing to build against" >&2
    echo "$packaged" | grep -E "^(error|error\[)" | head -6 >&2
    exit 1
fi

# **A STABLE work directory, not `mktemp -d`.**
#
# This used a fresh temporary directory, which gave cargo a fresh target
# directory, which meant the whole dependency tree was rebuilt on EVERY run:
# measured at 9m48s, in a gate that otherwise takes about two minutes. Nothing
# about the check needed that — it needed the tree to be the source files and
# the manifest to be rewritten, both of which a reused directory gives.
#
# Under `target/`, so it is not committed and `cargo clean` reaches it.
WORK="$ROOT/target/tarball-consumer"
mkdir -p "$WORK"

# **An exclude list, not `git ls-files`.**
#
# Asking git which files are tracked is the right question and was the wrong
# way to ask it here: the guard harness runs every check against a COPY of the
# tree, and a copy is not a git repository — so the listing came back empty and
# this failed on a clean tree, which the harness correctly reported as a guard
# that cannot be said to catch anything.
#
# Excluding build output works in the repository and in a copy of it, and
# `--delete` does what `--files-from` could not: a file removed from the tree
# is removed here too. Excluded directories are NOT deleted, which is what
# keeps `$WORK/target` — the reason this directory is reused at all.
rsync -a --delete \
      --exclude 'target/' --exclude 'node_modules/' --exclude '.git/' \
      "$ROOT/" "$WORK/"

python3 - "$WORK/Cargo.toml" "$SIBLING" <<'PY'
import sys, pathlib, re, subprocess, json

manifest, sibling = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])

versions = {
    p["name"]: p["version"]
    for p in json.loads(subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=sibling, capture_output=True, text=True, check=True).stdout)["packages"]
}

text = manifest.read_text()

# Every dependency pointing into the sibling becomes a registry dependency,
# keeping whatever else the entry said — `default-features = false` above all,
# since that is the whole dependency argument for this tower.
def rewrite(match):
    name, inner = match.group(1), match.group(2)
    version = versions.get(name)
    if version is None:
        sys.exit(f"check-against-tarballs: {name} is not a member of the sibling workspace")
    rest = re.sub(r'path\s*=\s*"[^"]*"\s*,?\s*', "", inner).strip().rstrip(",")
    if rest:
        return f'{name} = {{ version = "{version}", {rest} }}'
    return f'{name} = "{version}"'

text = re.sub(r'^([a-z0-9-]+)\s*=\s*\{([^}]*path\s*=\s*"\.\./galata-datawatch[^}]*)\}',
              rewrite, text, flags=re.M)

# **Refuse rather than pass** if anything still points at the checkout: a
# surviving path dependency would make this whole check a slow way of building
# the tree it already builds.
leftover = re.findall(r'^[a-z0-9-]+\s*=\s*\{[^}]*\.\./galata-datawatch[^}]*\}', text, re.M)
if leftover:
    sys.exit("check-against-tarballs: a path dependency on the sibling survived the rewrite, so "
             "this would have proved nothing:\n  " + "\n  ".join(leftover))

manifest.write_text(text)
PY

# Patched to the PACKAGED directories — the shipped files, reached the way a
# registry dependency reaches them.
# **Only the crates that PUBLISH.**
#
# `cargo package --workspace` stages every member, including the ones that
# never reach crates.io, and the first draft patched whatever it found on
# disk. Planting caught it: a tower made to depend on `galata-datawatch-vault`
# — `publish = false`, the crate that exists so the other four take no vault
# dependency — resolved happily against its staged copy, and the guard
# reported ok while describing the one fault it is for.
#
# Asked of cargo: `publish` is null when a crate may publish anywhere, and []
# when it may not.
publishable=$(cd "$SIBLING" && cargo metadata --no-deps --format-version 1 | python3 -c 'import json, sys
for pkg in json.load(sys.stdin)["packages"]:
    if pkg.get("publish") is None:
        print(pkg["name"] + "-" + pkg["version"])')

patches=()
for spec in $publishable; do
    dir="$SIBLING/target/package/$spec"
    [[ -f "$dir/Cargo.toml" ]] || {
        echo "against tarballs: $spec publishes and was not staged — the packaging step changed shape" >&2
        exit 2
    }
    patches+=(--config "patch.crates-io.${spec%-*}.path='$dir'")
done

if ! built=$(cd "$WORK" && cargo check --all-targets "${patches[@]}" 2>&1); then
    echo "against tarballs: the tower does not build against the crates that would ship" >&2
    echo "$built" | grep -E "^(error|error\[)" | head -10 >&2
    exit 1
fi

echo "against tarballs: ok. this tower's source compiles against galata-datawatch's PACKAGED crates as registry dependencies, default-features = false — not against the sibling checkout"
