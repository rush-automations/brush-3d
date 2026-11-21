#!/bin/sh
# CI self-check for the generated types. Fails if types/ops.d.ts differs
# from HEAD and reports the exact difference through workflow annotations,
# which are readable via the checks API (job logs are not accessible).
set -u

if git diff --quiet -- types/ops.d.ts; then
  echo "types/ops.d.ts is current"
  exit 0
fi

echo "::error file=bindings/wasm/types/ops.d.ts::types/ops.d.ts differs from HEAD after regeneration"

report() {
  # Escape workflow-command message data: % first, then CR. LF is avoided
  # by emitting one annotation per line.
  sed 's/%/%25/g; s/\r/%0D/g'
}

{
  echo "--- git version ---"
  git --version
  echo "--- relevant git config ---"
  git config --list | grep -iE 'crlf|eol|attr|filter|ignorecase|filemode' || echo "(none)"
  echo "--- git status --porcelain (types/) ---"
  git status --porcelain -- types/
  echo "--- ls -l types/ops.d.ts ---"
  ls -l types/ops.d.ts
  echo "--- index entry ---"
  git ls-files -s -- types/ops.d.ts
  echo "--- working tree blob hash ---"
  git hash-object types/ops.d.ts
  echo "--- head of working tree file ---"
  od -c types/ops.d.ts | head -4
  echo "--- git diff ---"
  git diff -- types/ops.d.ts | head -120
} | report | while IFS= read -r line; do
  echo "::error::$line"
done

exit 1
