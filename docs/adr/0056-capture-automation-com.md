# ADR-0056: Автоматический захват эталона strict-viewer через COM Word/Excel

**Статус:** принят  
**Дата:** 25.11.2026  
**Контекст:** политика strict-viewer [`baseline-policy.md`](../strict-viewer/baseline-policy.md) §3 п.5, [`certification-matrix.md`](../strict-viewer/certification-matrix.md) §2.2

## Контекст

Политика strict-viewer требовала снимать эталон **вручную** в Word Online /
Excel Online на машине подготовки baseline. Это давало два ограничения:

1. сорок документов корпуса (20 DOCX + 20 XLSX) плюс печатные режимы снимаются
   только человеком — часы монотонной работы на каждый baseline;
2. ручной захват нельзя воспроизвести: повторный прогон зависит от настроения
   оператора, а не от кода.

При этом в репозитории уже был работающий автоматический путь к настоящему
Microsoft Office — `scripts/oracle-msoffice.ps1` (`Export-WordPdf`,
`Export-ExcelPdf`): COM-автоматизация `Word.Application` / `Excel.Application`,
`ExportAsFixedFormat`, затем `pdftoppm`. Он использовался только для SSIM-оракула
и не был разрешён для эталонов.

## Решение

Принять решение `accept-capture-automation`: способом захвата эталона strict-viewer
наряду с ручным Office Online становится **COM-автоматизация локального
Microsoft Word/Excel** через `scripts/strict-viewer-references.ps1`.

Конвейер:

```text
corpus.json → Word/Excel COM ExportAsFixedFormat → PDF
            → pdftoppm -png -r 144 → PNG (RGB8/RGBA8, ≤ 4 МиБ)
            → drop <format>/<fixture-id>/<mode>/ + capture.json
            → pnpm visual-oracle reference import
```

**Конверт manifest.** Захват локальным Office нельзя честно записать как
Office Online, а схема до этого решения допускала только его (`provider:
"office-online"` был `const`): либо manifest врал бы о среде, либо эталон
становился невалидным. Поэтому `reference` расширяется:

| Поле | Значения |
|---|---|
| `reference.provider` | `office-online`, `office-desktop` |
| `reference.application` | `word-online`, `excel-online`, `word-desktop`, `excel-desktop` |
| `reference.captureMethod` | `manual`, `semi-automated`, `automated` |

Семейство приложения обязано совпадать с провайдером (`office-online` ⇒
`*-online`, `office-desktop` ⇒ `*-desktop`), а приложение — с форматом
документа (`docx` ⇒ `word-*`, `xlsx` ⇒ `excel-*`). Несовпадение — ошибка
валидации manifest, а не приведение к ближайшему допустимому значению.

## Последствия

- **Что не меняется.** Office по-прежнему не встраивается в продукт, тестовый
  runtime и comparator; в capture/diff runtime нет Office URL, сетевого вызова и
  зависимости от учётных данных Office. Это остаётся обязательной проверкой
  manifest (`baseline-policy.md` §4).
- **Что меняется.** Требование «эталон снимается только вручную в Office Online»
  снято. Ручной захват остаётся допустимым, но не обязательным.
- **Review не ослабляется.** Решение `accept-baseline` (два approvals,
  независимый повторный захват, запрет auto-update) в силе: скрипт создаёт
  **кандидата** на baseline, а принятие идёт отдельным baseline-only PR.
  Формально: `accept-capture-automation` меняет способ **съёмки**, а
  `accept-baseline` — способ **принятия**.
- **Режимы.** Из PDF-экспорта снимаются `docx.pdf` и `xlsx.print-preview`.
  `docx.print-layout` и `xlsx.normal` — экранные режимы веб-версии Office;
  скрипт их пропускает (нужен ручной захват или отдельное решение).
- **Среда.** Эталон снимается локальным Office на Windows; версия Office
  (`$word.Version`) и версия `pdftoppm` записываются в manifest, как требовала
  матрица §2.2.
- **Конверт и его потребители.** Ассет этого ADR — `manifest.schema.json`
  (новые enum и пара `application` в ветках формата). `scripts/strict-viewer-references.ps1`
  пишет `provider: office-desktop`, `application: word-desktop`/`excel-desktop`
  и `applicationVersion` из COM-версии Office, `captureMethod: automated`;
  поля описаны в `docs/strict-viewer/reference-capture.md` §3. Номер ADR,
  версия политики и матрицы не меняются.
- **CI.** Скрипт не входит в CI: нужны Windows и лицензия Office. Гейтом CI
  остаётся сравнение с уже принятым эталоном.

## Отклонённые варианты

- **Автоматизация Office Online (браузер).** Требует учётных данных Microsoft,
  разметка меняется без версии, эталон «поехал» бы молча и невоспроизводимо.
  Прямо запрещено §3 п.5 и §4.
- **LibreOffice `soffice --headless`.** Уже используется оракулами, работает на
  macOS/Linux, но эталоном станет LibreOffice, а матрица сертифицирует
  соответствие Microsoft Office. Расхождение с областью сертификации.

## Ссылки

- [`baseline-policy.md`](../strict-viewer/baseline-policy.md) — §1, §3, §4.
- [`certification-matrix.md`](../strict-viewer/certification-matrix.md) — §2.2, §8.
- [`docs/testing/strict-viewer-references.md`](../testing/strict-viewer-references.md) — инструкция.
- [`docs/testing/ms-office-oracle.md`](../testing/ms-office-oracle.md) — прежнее использование COM.
