#!/bin/sh
# Bumps the patch number in a MAJOR.MINOR.PATCH version file and prints the new version.
# Usage: scripts/bump-version.sh [FILE]   (default: VERSION)
set -eu

file="${1:-VERSION}"
current=$(tr -d '[:space:]' < "$file")

fail() {
    echo "bump-version: $1" >&2
    exit 1
}

case "$current" in
    *[!0-9.]* | '' | .* | *. | *..*) fail "'$current' is not MAJOR.MINOR.PATCH" ;;
esac
major=${current%%.*}
rest=${current#*.}
[ "$rest" != "$current" ] || fail "'$current' is not MAJOR.MINOR.PATCH"
minor=${rest%%.*}
patch=${rest#*.}
[ "$patch" != "$rest" ] || fail "'$current' is not MAJOR.MINOR.PATCH"
case "$patch" in
    *.*) fail "'$current' is not MAJOR.MINOR.PATCH" ;;
esac

new="$major.$minor.$((patch + 1))"
printf '%s\n' "$new" > "$file"
printf '%s\n' "$new"
