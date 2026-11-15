import { defineConfig } from 'vite';

export default defineConfig({
  root: 'src',
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    outDir: '../src-tauri/target/frontend-dist',
    emptyOutDir: true,
    target: 'esnext',
  },
});
