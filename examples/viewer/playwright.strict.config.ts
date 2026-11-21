import { defineConfig } from '@playwright/test';
import { readFileSync } from 'node:fs';

if (!process.env.STRICT_CAPTURE_PLAN) throw new Error('STRICT_CAPTURE_PLAN is required');
const plan = JSON.parse(readFileSync(process.env.STRICT_CAPTURE_PLAN, 'utf8'));
const port = 5179;

export default defineConfig({
  testDir: './e2e/strict',
  testMatch: 'capture.spec.ts',
  workers: 1,
  fullyParallel: false,
  reporter: [['list']],
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    viewport: plan.environment.viewportCss,
    deviceScaleFactor: plan.environment.deviceScaleFactor,
    locale: plan.environment.locale,
    timezoneId: plan.environment.timezone,
    colorScheme: plan.environment.colorScheme,
    browserName: 'chromium',
    launchOptions: { args: ['--font-render-hinting=none'] },
  },
  webServer: {
    command: `pnpm dev --port ${port} --strictPort`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
