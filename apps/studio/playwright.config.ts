import {defineConfig, devices} from '@playwright/test';

// Studio end-to-end tests against the real stack: Rust engine (IPC) +
// Puma/Sinatra control plane serving the built Studio. Processes are started
// by e2e/stack.ts; CI provides STEPHANIA_ENGINE_BIN.
export default defineConfig({
  testDir: 'e2e',
  testMatch: '*.e2e.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 60_000,
  reporter: [['list'], ['html', {open: 'never', outputFolder: 'playwright-report'}]],
  // CI uses the runner's preinstalled Chrome (STEPHANIA_PLAYWRIGHT_CHANNEL=chrome)
  // instead of downloading Playwright's Chromium on every run.
  use: {...devices['Desktop Chrome'], channel: process.env.STEPHANIA_PLAYWRIGHT_CHANNEL || undefined, trace: 'retain-on-failure'},
});
