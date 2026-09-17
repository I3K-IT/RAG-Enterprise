#!/usr/bin/env bash
#
# Assemble changelog.d/ fragments into a new version section in CHANGELOG.md.
#
#   scripts/release-changelog.sh 0.1.45
#
# Edits CHANGELOG.md in place and deletes the fragments it consumed. It does
# not commit and does not tag: read the diff first, because a changelog is
# read by people and this script only knows how to concatenate.
#
# See changelog.d/README.md for why entries live in separate files.

set -euo pipefail

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
    echo "usage: $0 <version>   e.g. $0 0.1.45" >&2
    exit 2
fi
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: '$VERSION' is not a version like 0.1.45" >&2
    exit 2
fi

cd "$(dirname "$0")/.."

CHANGELOG="CHANGELOG.md"
FRAGMENTS="changelog.d"
TODAY="$(date -u +%Y-%m-%d)"

if grep -q "^## \[$VERSION\]" "$CHANGELOG"; then
    echo "error: $CHANGELOG already has a [$VERSION] section" >&2
    exit 1
fi

# Keep a Changelog's order, with Performance where this project has been
# putting it. A fragment whose prefix is not in this list is an error rather
# than something quietly dropped into the wrong place.
SECTIONS=(added changed performance deprecated removed fixed security)

section_title() {
    case "$1" in
        added)       echo "Added" ;;
        changed)     echo "Changed" ;;
        performance) echo "Performance" ;;
        deprecated)  echo "Deprecated" ;;
        removed)     echo "Removed" ;;
        fixed)       echo "Fixed" ;;
        security)    echo "Security" ;;
    esac
}

is_known_section() {
    local candidate="$1"
    for s in "${SECTIONS[@]}"; do
        [[ "$s" == "$candidate" ]] && return 0
    done
    return 1
}

shopt -s nullglob
ALL_FRAGMENTS=()
for f in "$FRAGMENTS"/*.md; do
    [[ "$(basename "$f")" == "README.md" ]] && continue
    ALL_FRAGMENTS+=("$f")
done
shopt -u nullglob

if [[ ${#ALL_FRAGMENTS[@]} -eq 0 ]]; then
    echo "error: no fragments in $FRAGMENTS/ — nothing to release" >&2
    exit 1
fi

# Reject unknown prefixes before writing anything, so a typo cannot produce a
# half-assembled changelog that then has to be untangled by hand.
for f in "${ALL_FRAGMENTS[@]}"; do
    base="$(basename "$f")"
    prefix="${base%%-*}"
    if ! is_known_section "$prefix"; then
        echo "error: $f — '$prefix' is not a known section." >&2
        echo "       expected one of: ${SECTIONS[*]}" >&2
        exit 1
    fi
done

BODY="$(mktemp)"
trap 'rm -f "$BODY"' EXIT

{
    echo "## [$VERSION] - $TODAY"
    for s in "${SECTIONS[@]}"; do
        matches=()
        for f in "${ALL_FRAGMENTS[@]}"; do
            [[ "$(basename "$f")" == "$s"-* ]] && matches+=("$f")
        done
        [[ ${#matches[@]} -eq 0 ]] && continue

        echo
        echo "### $(section_title "$s")"
        # Sorted, so the order is reproducible rather than filesystem-dependent.
        while IFS= read -r f; do
            echo
            # Strip trailing blank lines from the fragment; the spacing between
            # entries is this script's job, not the contributor's.
            sed -e :a -e '/^\n*$/{$d;N;ba' -e '}' "$f"
        done < <(printf '%s\n' "${matches[@]}" | sort)
    done
    echo
    echo "---"
} > "$BODY"

# Insert directly below the [Unreleased] block's own separator, so the new
# section becomes the first released one and [Unreleased] keeps its pointer to
# changelog.d/.
python3 - "$CHANGELOG" "$BODY" <<'PY'
import sys, pathlib

changelog = pathlib.Path(sys.argv[1])
body = pathlib.Path(sys.argv[2]).read_text().rstrip("\n")
lines = changelog.read_text().split("\n")

start = next(i for i, l in enumerate(lines) if l.startswith("## [Unreleased]"))
# The first "---" after [Unreleased] closes it; the new section goes after.
sep = next(i for i in range(start + 1, len(lines)) if lines[i].strip() == "---")

out = lines[: sep + 1] + [""] + body.split("\n") + lines[sep + 1 :]
changelog.write_text("\n".join(out))
PY

for f in "${ALL_FRAGMENTS[@]}"; do
    rm -f "$f"
done

echo "CHANGELOG.md now has [$VERSION] - $TODAY, from ${#ALL_FRAGMENTS[@]} fragment(s)."
echo "Fragments removed. Review the diff, then commit."
