import { defineConfig, devices } from '@playwright/test';

/**
 * Порт закреплён: 5178 — исторический порт просмотрщика, по нему в документации
 * и в привычке. Сервер поднимает сам Playwright.
 */
const PORT = 5178;
// Автономный corpus запускается отдельным конфигом и требует capture plan.
const STRICT_CAPTURE_TESTS = '**/strict/**';

export default defineConfig({
  testDir: './e2e',
  timeout: 120_000,
  expect: { timeout: 20_000 },
  // WebKit на раннере периодически роняет вкладку («Target crashed»,
  // «browser has been closed») на разных тестах; Chromium и Firefox те же
  // сценарии проходят. Две попытки в CI дают свежий браузер, локально
  // ретраев нет — падение остаётся падением.
  retries: process.env.CI ? 2 : 0,
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
      testIgnore: [/dpr\.spec\.ts/, STRICT_CAPTURE_TESTS],
    },
    {
      name: 'firefox',
      use: { ...devices['Desktop Firefox'] },
      testIgnore: [/dpr\.spec\.ts/, STRICT_CAPTURE_TESTS],
    },
    {
      name: 'webkit',
      use: { ...devices['Desktop Safari'] },
      testIgnore: [/dpr\.spec\.ts/, STRICT_CAPTURE_TESTS],
      // На раннере WebKit падает не на конкретном тесте, а в случайном месте
      // цепочки: «Target crashed», таймауты `waitForFunction` и открытия
      // фикстуры. Двух глобальных попыток не хватает — набор падающих тестов
      // меняется от прогона к прогону; четыре дают свежий браузер и шанс
      // пройти весь спек.
      retries: process.env.CI ? 4 : 0,
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
