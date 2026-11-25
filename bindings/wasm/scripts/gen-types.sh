#!/bin/sh
# Wrapper around scripts/gen-types.mjs for callers invoking scripts/gen-types.sh directly.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec node "$here/gen-types.mjs" "$@"
