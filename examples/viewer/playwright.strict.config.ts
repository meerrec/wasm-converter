import { defineConfig } from '@playwright/test';
import { readFileSync } from 'node:fs';

if (!process.env.STRICT_CAPTURE_PLAN) throw new Error('STRICT_CAPTURE_PLAN is required');
const plan = JSON.parse(readFileSync(process.env.STRICT_CAPTURE_PLAN, 'utf8'));
const port = 5179;

export default defineConfig({
  testDir: './e2e/strict',
  testMatch: 'capture.spec.ts',
  workers: 1,
  timeout: 1_800_000,
  fullyParallel: false,
  reporter: [['list']],
  use: {
    baseURL: `http://localhost:${port}`,
    viewport: plan.environment.viewportCss,
    deviceScaleFactor: plan.environment.deviceScaleFactor,
    locale: plan.environment.locale,
    timezoneId: plan.environment.timezone,
    colorScheme: plan.environment.colorScheme,
    browserName: 'chromium',
    launchOptions: {
      executablePath: process.env.CHROMIUM_PATH || '/usr/bin/chromium',
      args: ['--font-render-hinting=none']
    },
  },
  webServer: {
    command: `corepack pnpm dev --port ${port} --strictPort`,
    url: `http://localhost:${port}`,
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
