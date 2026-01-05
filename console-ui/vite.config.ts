import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import monacoEditorPlugin from 'vite-plugin-monaco-editor';

const monacoPluginFactory =
  (monacoEditorPlugin as unknown as { default?: typeof monacoEditorPlugin }).default ?? monacoEditorPlugin;

export default defineConfig({
  define: {
    __INTLIFY_JIT_COMPILATION__: true,
    __INTLIFY_PROD_DEVTOOLS__: true,
  },
  plugins: [
    vue(),
    monacoPluginFactory({
      languageWorkers: ['editorWorkerService'],
    }),
  ],
  css: {
    preprocessorOptions: {
      scss: {
        api: 'modern-compiler',
      },
    },
  },
  server: {
    port: 15173,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:19309',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api/, ''),
      },
      '/ws': {
        target: 'ws://127.0.0.1:19309',
        ws: true,
      },
    },
  },
});
