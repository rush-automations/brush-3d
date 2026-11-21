#!/bin/sh
# Regenerate types/ops.d.ts from spec/ops.schema.json.
#
# This lives in a script file instead of the package.json "types" script so
# the banner comment is byte-identical no matter which tool runner executes
# it: `bun run` rewrites "npm" to "bun" inside package.json script strings,
# which silently changed the generated banner and broke the CI freshness
# check (`git diff --exit-code bindings/wasm/types`). Invoked via
# `sh scripts/gen-types.sh`, so no executable bit is required.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
wasm_dir=$(dirname "$here")

exec node "$wasm_dir/node_modules/json-schema-to-typescript/dist/src/cli.js" \
  --input "$wasm_dir/../../spec/ops.schema.json" \
  --output "$wasm_dir/types/ops.d.ts" \
  --bannerComment "/* Generated from spec/ops.schema.json by json-schema-to-typescript. Do not edit; run npm run types. */" \
  --additionalProperties false
