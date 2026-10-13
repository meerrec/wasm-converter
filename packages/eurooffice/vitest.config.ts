import { defineConfig } from 'vitest/config';

// Всё API пакета живёт на DOM (iframe, window, postMessage), поэтому среда
// jsdom нужна всем его тестам — не только одному файлу.
export default defineConfig({
  test: {
    environment: 'jsdom',
  },
});
