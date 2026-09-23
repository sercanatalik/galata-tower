#!/usr/bin/env bash
#
# Every guard, proved able to fail.
#
# A guard that has only ever passed is evidence of nothing. TWICE in this
# repository's short history a guard reported success while doing nothing --
# `check-no-float-money.sh` scanned a directory that did not exist, and one
# repository over a feature matrix built `--lib` only, so the single target
# that could not compile was the one it never compiled. Both were found by
# running a plant, not by reading the script.
#
# So every guard answers three verbs, and this runs all of them:
#
#   check   <root>    exit 0 clean, exit 1 on a violation
#   plant   <root>    create exactly one violation of THIS guard
#   targets <root>    print every path the guard depends on
#
# and a guard may answer two more:
#
#   expect  <root>    print what its planted failure must NAME, one per line
#   plants  <root>    print the names of several plants, one per line; `plant`
#                     and `expect` then read the one to use from
#                     GALATA_GUARD_PLANT. A guard that does not answer has one.
#
# EXPECT IS THE LOAD-BEARING PART. A suite whose criterion is only "the guard
# exited non-zero" CANNOT TELL A CAUGHT MUTANT FROM A BROKEN GUARD: a script
# with a syntax error exits non-zero too and would be counted as working.
# Where a failure has a stable wording worth pinning, the guard states it and
# this checks the message says it.
#
# Each guard is checked on its own fresh copies: clean (must pass), then once
# per plant (each must fail). Clean is checked FIRST and reported distinctly,
# because a guard that always fails is otherwise read as one that works.
#
# Plants only ever touch a copy under this suite's scratch root.
#
# Carried from galata-vault's scripts/test-guards.sh.
#
# Usage: test-guards.sh [root]

set -uo pipefail

ROOT="$(cd "${1:-$(dirname "${BASH_SOURCE[0]}")/..}" && pwd)"
# **A STABLE path, and this is the whole reason the shared build directory
# works.**
#
# This was `mktemp -d .../tower-guards-XXXXXX`, removed on exit. The copies
# went; the artifacts they compiled did not. Cargo keys a workspace crate's
# fingerprint on its PATH, so a new scratch directory every run meant a fresh
# set of artifacts every run, matchable by nothing and cleaned by nobody —
# 6.2 GB of them, inside the directory `Swatinem/rust-cache` saves in CI.
#
# It also explains a number that looked like noise: this harness is 38s warm
# and 120s with that directory cold, and cold was every run.
#
# Under `target/` because it is build output, `cargo clean` reaches it, and
# every copy already excludes it.
SCRATCH="$ROOT/target/guard-scratch"
mkdir -p "$SCRATCH"

failures=()

# THIS TREE IS NOT RELOCATABLE, and the harness found that out on its first
# run: `Cargo.toml` takes galata-datawatch by PATH -- `../galata-datawatch/...`
# -- because that workspace publishes nothing yet, so a copy in a scratch
# directory has no sibling to resolve against and every guard that builds fails
# on a clean copy.
#
# So a copy is made INSIDE a parent that carries a symlink to the real sibling,
# and `../galata-datawatch` resolves from the copy exactly as it does here. When
# galata-datawatch publishes and those become registry dependencies, the symlink
# stops being needed and this comment is how somebody will know it can go.
SIBLING="$(cd "$ROOT/.." && pwd)/galata-datawatch"

copy_tree() {
    local parent="$1"
    local dest="$parent/galata-tower"
    # **Wiped, not overwritten.** This extracted a tar over whatever was there,
    # which was safe only because the parent was new every run. With a stable
    # path it is not: a file deleted from the repository would survive here and
    # a guard would be checking a tree that does not exist — and a plant left
    # by an interrupted run would be read as this run's tree, which is the
    # class of mistake this harness exists to catch rather than commit.
    rm -rf "$dest"
    mkdir -p "$dest"
    ln -sfn "$SIBLING" "$parent/galata-datawatch"
    tar -cf - -C "$ROOT" \
        --exclude=./target --exclude=./.git --exclude=./ui/node_modules . \
        | tar -xf - -C "$dest"
    # `ui/node_modules` is excluded because it is tens of thousands of files
    # per guard. The guards that need it install it themselves, frozen —
    # and it must NOT be symlinked in instead: `pnpm install` wants to
    # replace a modules directory it did not create, and aborts without a
    # TTY, which turned check-dist-drift red on a clean copy when that was
    # tried.
    echo "$dest"
}

# A plant mutates by definition, so it runs only on a directory that is under
# SCRATCH, is not a symlink, and actually holds this workspace.
safe_to_plant() {
    local tree="$1"
    [[ -L "$tree" ]] && return 1
    case "$tree" in "$SCRATCH"/*) ;; *) return 1 ;; esac
    [[ -f "$tree/Cargo.toml" && -d "$tree/crates/galata-tower" ]]
}

guards=0
planted_count=0
# **One build directory for every copy.**
#
# This harness copies the tree for each guard's clean check and again for each
# plant — eighteen copies here. Each copy is a new path, so each used to get
# its own `target/` and compile 245 crates from nothing. Measured, with the
# copy split out: 59s across eight clean checks and 42s across ten plants, of
# which the copying itself was **6s**. The cost was never the copying.
#
# Cargo reuses across paths when the target directory is shared — measured,
# because the opposite is easy to assume and the previous change's design
# assumed it:
#
#   copy A, cold                                  11s
#   copy B, different path, same target dir        0s
#
# A dependency's fingerprint is keyed on the package and its inputs, not on
# which workspace asked for it.
#
# **This requires the harness to stay serial.** Cargo locks a target directory,
# so copies sharing one must not run at the same time. Nothing here runs them
# concurrently; whoever adds a `&` needs to give each its own directory again.
export CARGO_TARGET_DIR="$ROOT/target/guard-harness"

for guard in "$ROOT"/scripts/check-*.sh; do
    [[ -e "$guard" ]] || continue
    name="$(basename "$guard" .sh)"
    [[ "$name" == "check-all" ]] && continue
    guards=$((guards + 1))

    # Every path the guard names must exist, or it is guarding a ghost.
    while IFS= read -r target; do
        [[ -n "$target" ]] || continue
        [[ -e "$ROOT/$target" ]] || failures+=("$name: target $target does not exist")
    done < <("$guard" targets "$ROOT" 2>/dev/null)

    clean="$(copy_tree "$SCRATCH/$name-clean")"
    if ! "$guard" check "$clean" >/dev/null 2>&1; then
        failures+=("$name: fails on a clean copy of the tree — it cannot be said to catch anything")
        continue
    fi

    plants=$("$guard" plants "$ROOT" 2>/dev/null) || plants=""
    [[ -n "$plants" ]] || plants="default"
    for plant in $plants; do
        label="$name"
        [[ "$plant" == default ]] || label="$name ($plant)"
        plant_began=$SECONDS
        planted="$(copy_tree "$SCRATCH/$name-planted-$plant")"
        if ! safe_to_plant "$planted"; then
            failures+=("$label: refused to plant into $planted")
            continue
        fi
        planted_count=$((planted_count + 1))
        # **What this plant cost, split.** The gate's timing named this harness
        # as its largest part; the next question is whether that is the tree
        # copy above or the guard runs below, and it is cheaper to print the
        # answer than to instrument this by hand again.
        copied=$(( SECONDS - plant_began ))
        GALATA_GUARD_PLANT="$plant" "$guard" plant "$planted" >/dev/null 2>&1
        if out=$("$guard" check "$planted" 2>&1); then
            failures+=("$label: PASSES with its own violation planted; it cannot fail")
        elif expected=$(GALATA_GUARD_PLANT="$plant" "$guard" expect "$ROOT" 2>/dev/null); then
            # Failing is not enough: it must fail for the PLANTED reason.
            while IFS= read -r needle; do
                [[ -n "$needle" ]] || continue
                grep -qF -- "$needle" <<<"$out" \
                    || failures+=("$label: its planted failure does not name \"$needle\"")
            done <<<"$expected"
        fi
        printf '  %s: %ss (%ss copying the tree)\n' "$label" "$(( SECONDS - plant_began ))" "$copied"
    done
done

if (( guards == 0 )); then
    echo "guards: no scripts/check-*.sh found; refusing to report an empty suite as ok" >&2
    exit 1
fi

if (( ${#failures[@]} > 0 )); then
    echo "guards: FAILED" >&2
    printf '  %s\n' "${failures[@]}" >&2
    exit 1
fi

echo "guards: ok. $guards guard(s) and $planted_count plant(s): each passes clean and fails with each of its violations planted"
