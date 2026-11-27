import {fileURLToPath} from 'node:url';
import {defineConfig} from 'vitest/config';
// Same aliases as tsconfig.json (Expo CLI reads them from there).
export default defineConfig({
  resolve: {alias: {'@/': fileURLToPath(new URL('./src/', import.meta.url)), '@modules/': fileURLToPath(new URL('./modules/', import.meta.url))}},
  test: {include: ['src/**/*.test.ts']},
});
