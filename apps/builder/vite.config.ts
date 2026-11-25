import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

export default defineConfig({
  base: './',
  resolve: {
    alias: {
      'brush-wasm': fileURLToPath(new URL('../../bindings/wasm/pkg/brush_wasm.js', import.meta.url)),
      '@brush/web-editor': fileURLToPath(new URL('../../packages/web-editor/src/index.ts', import.meta.url)),
    },
    dedupe: ['three'],
  },
  server: {
    fs: { allow: ['../..'] },
  },
});
