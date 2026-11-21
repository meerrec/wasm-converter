# Автономный визуальный оракул

## Текущая граница

`test-fixtures/strict-viewer/corpus.json` фиксирует 20 DOCX и 20 XLSX как **кандидатов**. Их SHA-256 проверены, но `referenceStatus` остаётся `pending`: Office Online PNG/PDF/manifest ещё не получены. Это не accepted baseline и не доказательство визуального соответствия.

Синтетические PNG в `tools/png.test.mjs` проверяют только decoder/diff/exit codes. Они не входят в документный корпус.

## Проверки без сети

```bash
node tools/strict-viewer/verify-corpus.mjs
node --test tools/strict-viewer/png.test.mjs
```

Comparator принимает только 8-bit non-interlaced RGB/RGBA PNG, как требует manifest contract. RGB delta до 2 включительно проходит; alpha должна совпадать точно. Любой другой непокрытый маской пиксель даёт `visual-diff`. SSIM вычисляется по luminance в окнах 8×8 и служит только диагностикой.

Коды выхода: `0` — match; `1` — visual/page mismatch; `2` — missing reference/capture error; `3` — harness/schema/artifact failure.

## Подготовка Office references

Office Online используется только человеком или отдельным semi-automated capture вне продукта и обычного CI. Для каждого документа нужен manifest `1.0.0`, все выбранные PNG без JPEG-перекодирования и exported PDF. Динамические маски должны иметь причину и оставаться внутри изображения. Повторный захват сравнивается с первым до independent QA review.

PNG и PDF хранятся через Git LFS по правилам `.gitattributes`. Один PNG не должен превышать 4 MiB; начальный PNG/PDF corpus — 250 MiB. Превышение останавливает приёмку, а не вызывает автоматическое сжатие.

## Actual capture

Соберите существующие WASM modules и поднимите dependencies `examples/viewer`. Создайте план только из настоящих Office manifests:

```bash
node tools/strict-viewer/build-capture-plan.mjs /tmp/strict-plan.json \
  test-fixtures/strict-viewer/references/*/*/manifest.json
STRICT_CAPTURE_PLAN=/tmp/strict-plan.json \
STRICT_CAPTURE_OUTPUT=/tmp/strict-actual \
pnpm --dir examples/viewer exec playwright test --config playwright.strict.config.ts
```

Захват использует Chromium, существующий viewer bridge и worker/WASM path. DOCX PDF и XLSX Print Preview экспортируются собственным viewer и растрируются локальным `pdftoppm` при 144 DPI. Продукт не вызывает Office. Для zoom, отличного от 100 %, harness явно завершает capture с ошибкой до появления проверенного zoom adapter.

Каждый capture пишет `actual.json`; ошибка одного документа не останавливает остальные. Пустой PNG никогда не заменяет failed capture. Два прогона нужно сравнить побайтово в одной зафиксированной среде до приёмки воспроизводимости.

## Offline corpus report

```bash
node tools/strict-viewer/compare-corpus.mjs /tmp/strict-plan.json /tmp/strict-actual /tmp/strict-report
```

Команда читает только локальные reference/actual artifacts и создаёт `report.json` и `report.html`. Худшие captures идут первыми; reference, actual и diff показываются рядом. Число DOCX captures и порядок XLSX captures проверяются отдельно.

Для одной пары доступен низкоуровневый CLI:

```bash
node tools/strict-viewer/cli.mjs --reference reference.png --actual actual.png --output /tmp/pair-report
```

## Явное принятие baseline

Обычный capture/diff ничего не пишет в references. Кандидат создаётся только отдельной командой, с digest и двумя разными approvals; independent QA не может быть автором захвата:

```bash
node tools/strict-viewer/accept-baseline.mjs \
  --actual /tmp/strict-actual/docx/page-1/actual.png \
  --candidate test-fixtures/strict-viewer/candidates/docx/page-1.png \
  --approval approvals.json --accept-visual yes
```

Существующий файл не перезаписывается. После этого кандидат проходит baseline-only PR; команда сама не меняет corpus status и не делает reference accepted.
