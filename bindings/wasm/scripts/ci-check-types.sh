#!/bin/sh
# CI self-check for the generated types. Fails if types/ops.d.ts differs
# from HEAD and reports the exact difference through workflow annotations
# (job logs are not accessible; annotations are kept only up to ~10, so
# this emits six dense lines, most diagnostic first).
set -u

if git diff --quiet -- types/ops.d.ts; then
  echo "types/ops.d.ts is current"
  exit 0
fi

esc() { sed 's/%/%25/g; s/\r/%0D/g'; }

{
  echo "A status=[$(git status --porcelain -- types/ | tr '\n' ';')]"
  echo "B ls=[$(ls -l types/ops.d.ts)]"
  echo "C index=[$(git ls-files -s -- types/ops.d.ts)]"
  echo "D workhash=[$(git hash-object types/ops.d.ts)] headblob=[$(git rev-parse HEAD:bindings/wasm/types/ops.d.ts)]"
  echo "E summary=[$(git diff --summary -- types/ops.d.ts | tr '\n' ';')] numstat=[$(git diff --numstat -- types/ops.d.ts | tr '\n' ';')]"
  echo "F diffhead=$(git diff -- types/ops.d.ts | head -8 | tr '\n' '|')"
} | esc | while IFS= read -r line; do
  echo "::error::$line"
done

exit 1
