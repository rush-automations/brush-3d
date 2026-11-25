// Regenerate types/ops.d.ts from spec/ops.schema.json across Linux, macOS, and Windows.
// Lives in a standalone Node script so `bun run` cannot rewrite "npm" -> "bun" in the
// banner comment and CRLF cannot creep in on Windows checkouts.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptsDir = dirname(fileURLToPath(import.meta.url));
const wasmDir = join(scriptsDir, '..');
const cliPath = join(wasmDir, 'node_modules', 'json-schema-to-typescript', 'dist', 'src', 'cli.js');
const inputPath = join(wasmDir, '..', '..', 'spec', 'ops.schema.json');
const outputPath = join(wasmDir, 'types', 'ops.d.ts');

execFileSync(
  process.execPath,
  [
    cliPath,
    '--input',
    inputPath,
    '--output',
    outputPath,
    '--bannerComment',
    '/* Generated from spec/ops.schema.json by json-schema-to-typescript. Do not edit; run npm run types. */',
    '--additionalProperties',
    'false',
  ],
  { cwd: wasmDir, stdio: 'inherit' },
);

const normalized = readFileSync(outputPath, 'utf8').replace(/\r\n/g, '\n');
writeFileSync(outputPath, normalized, 'utf8');
