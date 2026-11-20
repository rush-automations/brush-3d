import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

// The engine comes straight from the wasm-pack output, so a rebuilt engine shows up on reload.
export default defineConfig({
  resolve: {
    alias: {
      'brush-wasm': fileURLToPath(new URL('../../bindings/wasm/pkg/brush_wasm.js', import.meta.url)),
    },
  },
  server: {
    fs: { allow: ['../..'] },
  },
});
