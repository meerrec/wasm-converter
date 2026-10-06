# Эпик E — кросс-браузерность

- Спринт 6 «Базовый PDF» · ветка `sprint/6-basic-pdf`
- Коммиты: e1117f6 (проекты Firefox/WebKit), 2931377 (CI: браузеры и
  pdf-утилиты), f79e788 (кнопка экспорта + capability detection)

## Что сделано

**E1. Матрица браузеров** (`examples/viewer-xlsx/playwright.config.ts`):
добавлены проекты `firefox` и `webkit`; `dpr.spec.ts` исключён из них через
`testIgnore` и идёт только в отдельном проекте `chromium-dpr2` (`testMatch`).
Итого 4 проекта вместо прежних двух.

**E2. Прогон расширенной матрицы.** Локальный полный прогон — 22/22:
chromium 7, firefox 7, webkit 7, chromium-dpr2 1. Состав: `viewer.spec.ts`
(5 тестов), `pdf.spec.ts` (1), `lazy-pdf.spec.ts` (1) в каждом из трёх
браузеров плюс dpr-сценарий в своём проекте.

**E3. Capability detection** (f79e788). Вместо fallback-рендеринга на main
thread — внятная ошибка при отсутствии `transferControlToCanvas`. Основание:
OffscreenCanvas в воркере есть в Chrome 69+, Firefox 105+, Safari 16.4+,
поэтому второй путь рендеринга не окупается.

**E4. e2e-экспорт PDF** (`examples/viewer-xlsx/e2e/pdf.spec.ts`): скачивание
файла `text-cyrillic-wrap.pdf`, проверка сигнатуры `%PDF-` и непустого
содержимого. Рядом `lazy-pdf.spec.ts` — проверка ленивой загрузки PDF-модуля
(ни одного запроса до клика, ровно один после, повторный экспорт из кэша).

**CI** (2931377): `playwright install --with-deps chromium firefox webkit`;
в rust-джобу добавлены `qpdf` и `poppler-utils` для внешних проверок F3.

## Метрики

- Проектов Playwright — 4; тестов в полном прогоне — 22 (7 / 7 / 7 / 1).
- CI ставит 3 браузерных движка; rust-джоба — 2 pdf-утилиты.
- Экспорт PDF и ленивость покрыты в chromium, firefox и webkit (7 тестов на
  браузер); dpr-сценарий — 1 тест в своём проекте.
- 3 коммита эпика.

## Отклонения от плана

- E3 урезан: fallback рендеринга на main thread не делается, только
  capability detection с ошибкой; fallback — в бэклоге (решение архитектора,
  пункт 5 плана).
- Ветка ошибки «нет `transferControlToCanvas`» e2e не покрыта: без мока
  браузера она не воспроизводится. Покрытие — только ручное/при ревью.
