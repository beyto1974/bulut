#!/bin/sh
# Tests for bump-version.sh. Run: sh scripts/test-bump-version.sh
set -u

here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failures=0

expect_bump() { # input expected
    printf '%s' "$1" > "$tmp/v"
    out=$(sh "$here/bump-version.sh" "$tmp/v" 2>&1)
    file=$(cat "$tmp/v")
    if [ "$out" = "$2" ] && [ "$file" = "$2" ]; then
        echo "ok   : '$1' -> $2"
    else
        echo "FAIL : '$1' gave output '$out' and file '$file', wanted $2"
        failures=$((failures + 1))
    fi
}

expect_refusal() { # input
    printf '%s' "$1" > "$tmp/v"
    if sh "$here/bump-version.sh" "$tmp/v" > /dev/null 2>&1; then
        echo "FAIL : '$1' should be refused"
        failures=$((failures + 1))
    elif [ "$(cat "$tmp/v")" != "$1" ]; then
        echo "FAIL : '$1' was refused but the file changed"
        failures=$((failures + 1))
    else
        echo "ok   : '$1' is refused and left alone"
    fi
}

expect_bump "0.1.0" "0.1.1"
expect_bump "0.1.0
" "0.1.1"
expect_bump "  1.2.9  " "1.2.10"
expect_bump "10.20.99" "10.20.100"
expect_refusal "1.2"
expect_refusal "1.2.3.4"
expect_refusal "v1.2.3"
expect_refusal "1.2.x"
expect_refusal ""
expect_refusal "1..3"
expect_refusal ".1.2"

[ "$failures" -eq 0 ] && echo "all passed" || { echo "$failures failed"; exit 1; }
