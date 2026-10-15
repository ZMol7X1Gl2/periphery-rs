#!/usr/bin/env bash
# Release helpers for CHANGELOG.md (Keep a Changelog format), used by
# .github/workflows/release.yml and usable locally.
#
#   release.sh next-version <patch|minor|major> [exact]
#       Prints the next version. `exact` (e.g. 0.3.0-rc.1) wins when given.
#       patch on a pre-release (0.1.1-rc.2) releases it (0.1.1).
#   release.sh promote <version>
#       Turns "## [Unreleased]" into "## [<version>] - <today>", starts a new
#       empty Unreleased section and updates the compare links. Fails when
#       Unreleased has no entries.
#   release.sh section <version|Unreleased>
#       Prints the body of that section without its heading.
set -euo pipefail

FILE=CHANGELOG.md

current_version() {
    sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1
}

repo_url() {
    sed -n 's/^repository = "\(.*\)"$/\1/p' Cargo.toml | head -1
}

next_version() {
    local bump="$1" exact="${2:-}"
    if [[ -n "$exact" ]]; then
        echo "${exact#v}"
        return
    fi
    local cur base pre major minor patch
    cur="$(current_version)"
    base="${cur%%-*}"
    pre=""
    [[ "$cur" == *-* ]] && pre="${cur#*-}"
    IFS=. read -r major minor patch <<<"$base"
    case "$bump" in
        patch) if [[ -n "$pre" ]]; then echo "$base"; else echo "$major.$minor.$((patch + 1))"; fi ;;
        minor) echo "$major.$((minor + 1)).0" ;;
        major) echo "$((major + 1)).0.0" ;;
        *) echo "unknown bump: $bump" >&2; exit 1 ;;
    esac
}

# Body of "## [<name>]" up to the next "## [" heading or the link list,
# with leading and trailing blank lines removed.
section() {
    local name="${1#v}"
    awk -v h="## [$name]" '
        index($0, h) == 1 { on = 1; next }
        on && (/^## \[/ || /^\[[^]]+\]: /) { exit }
        on { print }
    ' "$FILE" | sed -e '/./,$!d' | awk '{ lines[NR] = $0 } END { n = NR; while (n > 0 && lines[n] == "") n--; for (i = 1; i <= n; i++) print lines[i] }'
}

promote() {
    local version="${1#v}" today url prev
    if ! grep -q '^## \[Unreleased\]' "$FILE"; then
        echo "$FILE has no '## [Unreleased]' section" >&2
        exit 1
    fi
    if ! section Unreleased | grep -q '^- '; then
        echo "nothing under Unreleased in $FILE: describe the changes there first" >&2
        exit 1
    fi
    if grep -q "^## \[$version\]" "$FILE"; then
        echo "$FILE already has a section for $version" >&2
        exit 1
    fi
    today="$(date -u +%Y-%m-%d)"
    url="$(repo_url)"
    prev="$(sed -n 's|^\[Unreleased\]: .*/compare/\(.*\)\.\.\.HEAD$|\1|p' "$FILE")"
    awk -v v="$version" -v d="$today" -v url="$url" -v prev="$prev" '
        /^## \[Unreleased\]/ { print; print ""; print "## [" v "] - " d; next }
        /^\[Unreleased\]: / {
            print "[Unreleased]: " url "/compare/v" v "...HEAD"
            if (prev != "") print "[" v "]: " url "/compare/" prev "...v" v
            else print "[" v "]: " url "/releases/tag/v" v
            next
        }
        { print }
    ' "$FILE" >"$FILE.new"
    mv "$FILE.new" "$FILE"
}

cmd="${1:-}"
shift || true
case "$cmd" in
    next-version) next_version "$@" ;;
    promote) promote "$@" ;;
    section) section "$@" ;;
    *) sed -n '2,14p' "$0"; exit 1 ;;
esac
