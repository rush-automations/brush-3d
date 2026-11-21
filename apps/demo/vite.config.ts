import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

// The engine comes straight from the wasm-pack output, so a rebuilt engine shows up on reload.
// `base: "./"` keeps asset links working when the demo is served from a sub-path (GitHub Pages).
export default defineConfig({
  base: './',
  resolve: {
    alias: {
      'brush-wasm': fileURLToPath(new URL('../../bindings/wasm/pkg/brush_wasm.js', import.meta.url)),
    },
  },
  server: {
    fs: { allow: ['../..'] },
  },
});
