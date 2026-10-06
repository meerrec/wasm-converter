import { defineConfig, devices } from '@playwright/test';

/** Порт отдельный от остальных примеров: сервер поднимает сам Playwright. */
const PORT = 5178;

export default defineConfig({
  testDir: './e2e',
  timeout: 120_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  workers: 1,
  reporter: process.env.CI ? 'github' : 'list',
  use: {
    // Именно `localhost`: Vite слушает IPv6, и 127.0.0.1 не отвечает.
    baseURL: `http://localhost:${PORT}`,
    trace: 'retain-on-failure',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
      // DPR-сценарию нужен свой `deviceScaleFactor`: он идёт отдельным проектом.
      testIgnore: /dpr\.spec\.ts/,
    },
    {
      name: 'firefox',
      use: { ...devices['Desktop Firefox'] },
      testIgnore: /dpr\.spec\.ts/,
    },
    {
      name: 'webkit',
      use: { ...devices['Desktop Safari'] },
      testIgnore: /dpr\.spec\.ts/,
    },
    {
      name: 'chromium-dpr2',
      use: { ...devices['Desktop Chrome'], deviceScaleFactor: 2 },
      testMatch: /dpr\.spec\.ts/,
    },
  ],
  webServer: {
    command: `pnpm dev --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
  },
});
