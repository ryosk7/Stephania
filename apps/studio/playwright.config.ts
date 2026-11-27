import {defineConfig, devices} from '@playwright/test';

// Studio end-to-end tests against the real stack: Rust engine (IPC) +
// Puma/Sinatra control plane serving the built Studio. Processes are started
// by e2e/stack.ts; CI provides T3BOX_ENGINE_BIN.
export default defineConfig({
  testDir: 'e2e',
  testMatch: '*.e2e.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 60_000,
  reporter: [['list'], ['html', {open: 'never', outputFolder: 'playwright-report'}]],
  use: {...devices['Desktop Chrome'], trace: 'retain-on-failure'},
});
