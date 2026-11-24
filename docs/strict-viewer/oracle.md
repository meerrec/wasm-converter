# Автономный визуальный оракул

## Текущая граница

`test-fixtures/strict-viewer/corpus.json` фиксирует 20 DOCX и 20 XLSX как **кандидатов**. Их SHA-256 проверены; Office Online PNG/PDF и 84 manifest получены отдельной подготовкой references. `referenceStatus` остаётся `pending`: это не accepted baseline и не доказательство визуального соответствия. Для принятия всё ещё нужны повторный capture, independent QA и два approvals.

Синтетические PNG в `tools/strict-viewer/png.test.mjs` проверяют только decoder/diff/exit codes. Они не входят в документный корпус.

## Проверки без сети

```bash
python3 -m pip install -r tools/strict-viewer/requirements.txt
node tools/strict-viewer/verify-corpus.mjs
node --test tools/strict-viewer/png.test.mjs
python3 tools/strict-viewer/validate_references.py
```

Для изоляции Python-зависимостей можно создать виртуальное окружение: `python3 -m venv /tmp/strict-viewer-venv`, затем вызывать `/tmp/strict-viewer-venv/bin/python` в командах установки и проверки references.

Comparator принимает только 8-bit non-interlaced RGB/RGBA PNG, как требует manifest contract. RGB delta до 2 включительно проходит; alpha должна совпадать точно. Любой другой непокрытый маской пиксель даёт `visual-diff`. SSIM вычисляется по luminance в окнах 8×8 и служит только диагностикой.

Коды выхода: `0` — match; `1` — visual/page mismatch; `2` — missing reference/capture error; `3` — harness/schema/artifact failure.

## Подготовка Office references

Office Online используется только человеком или отдельным semi-automated capture вне продукта и обычного CI. Для каждого документа нужен manifest `1.0.0`, все выбранные PNG без JPEG-перекодирования и exported PDF. Динамические маски должны иметь причину и оставаться внутри изображения. Повторный захват сравнивается с первым до independent QA review.

Сохранённые raw captures преобразуются отдельной командой, которая не вызывается обычным build/test и не перезаписывает repository references без явного `--output`:

```bash
node tools/strict-viewer/capture-word-online.cjs /tmp/office-word
node tools/strict-viewer/capture-excel-online.cjs /tmp/office-excel
python3 tools/strict-viewer/prepare_references.py \
  --word /tmp/office-word --excel /tmp/office-excel \
  --output /tmp/strict-references
```

Preparation проверяет SHA-256 исходников и Word PDF, сохраняет embedded-font metadata, нормализует PNG в RGBA8 без изменения пикселей и растрирует Word PDF через `pdftoppm -r 144`. Версия Chromium определяется из исполняемого файла, а версия Office берётся только из фактического `x-officeversion` (Word) или `BuildVersion` (Excel); fallback с выдуманной версией запрещён. `OFFICE_VERSION` позволяет потребовать конкретную Word build при повторном захвате.

Excel Print Preview возвращает не больше 100 страниц на диапазон. Capture переключает `#pageRangeInput` на `1-100`, `101-200` и далее, а номер видимой страницы внутри диапазона вычисляет как `(pageNumber - 1) % 100 + 1`. Поэтому дальние anchors нельзя проверять старым захватом, который после страницы 100 повторял последний видимый кадр.

Normal capture ждёт стабильный canvas и явно фокусирует `A1`. Office всё же может чередовать оттенок служебного gutter строк; только прямоугольник `x=23, y=35, width=15, height=550` помечается `office-row-gutter-focus`/`documented-nondeterminism`. Ячейки и печатные страницы не маскируются.

Два prepared corpus сравниваются отдельной offline-командой:

```bash
python3 tools/strict-viewer/compare-reference-runs.py \
  /tmp/strict-references-run1 /tmp/strict-references-run2 \
  /tmp/reference-repeatability.json
```

Проверяются capture IDs, размеры и decoded pixels вне одинаковых declared masks. Различия Office builds и PDF bytes записываются отдельно: экспортированные Word PDF не обязаны быть побайтово стабильными между сессиями, поэтому визуальную повторяемость PDF проверяют по raster pages.

PNG и PDF хранятся через Git LFS по правилам `.gitattributes`. Один PNG не должен превышать 4 MiB; начальный PNG/PDF corpus — 250 MiB. Превышение останавливает приёмку, а не вызывает автоматическое сжатие.

## Actual capture

Соберите существующие WASM modules и поднимите dependencies `examples/viewer`. Создайте план только из настоящих Office manifests:

```bash
find test-fixtures/strict-viewer/references -name manifest.json -print0 | \
  xargs -0 node tools/strict-viewer/build-capture-plan.mjs /tmp/strict-plan.json
STRICT_CAPTURE_PLAN=/tmp/strict-plan.json \
STRICT_CAPTURE_OUTPUT=/tmp/strict-actual \
pnpm --dir examples/viewer exec playwright test --config playwright.strict.config.ts
```

Если порт `5179` уже занят, задайте свободный `STRICT_VIEWER_PORT=<port>` перед командой.

Захват использует Chromium, существующий viewer bridge и worker/WASM path. DOCX PDF и XLSX Print Preview экспортируются собственным viewer и растрируются локальным `pdftoppm` при 144 DPI; DOCX `print-layout` использует 96 DPI, чтобы совпасть с Office print PNG 794×1123. Продукт не вызывает Office. Для zoom, отличного от 100 %, harness явно завершает capture с ошибкой до появления проверенного zoom adapter.

Каждый capture пишет `actual.json`; ошибка одного документа не останавливает остальные. Пустой PNG никогда не заменяет failed capture. Недостающая actual page сохраняется как `page-mismatch`, а не как harness failure. Два прогона в одной зафиксированной среде сравниваются по status, page count, decoded PNG pixels и PDF bytes:

```bash
node tools/strict-viewer/compare-capture-runs.mjs \
  /tmp/strict-actual-run1 /tmp/strict-actual-run2 \
  /tmp/strict-repeatability.json
```

## Offline corpus report

```bash
node tools/strict-viewer/compare-corpus.mjs /tmp/strict-plan.json /tmp/strict-actual /tmp/strict-report
```

Команда читает только локальные reference/actual artifacts и создаёт `report.json` и `report.html`. Худшие captures идут первыми; reference, actual и diff показываются рядом. Число DOCX captures и порядок XLSX captures проверяются отдельно.

Triage выполняется поверх того же offline report и capture plan:

```bash
python3 tools/strict-viewer/classify_report.py \
  --plan /tmp/strict-plan.json --report /tmp/strict-report/report.json \
  --output /tmp/strict-report
```

Classifier создаёт `classification.json` и `classification.html` с counts, examples, worst captures и static OOXML analysis. Категории `parse/model/layout/font/render/harness/reference` являются гипотезами triage, а не доказанными root causes.

Для одной пары доступен низкоуровневый CLI:

```bash
node tools/strict-viewer/cli.mjs --reference reference.png --actual actual.png --output /tmp/pair-report
```

## GitHub Actions

Workflow `.github/workflows/stage-1-oracle.yml` запускает на `stage-1/**`, pull requests и вручную три группы проверок: integrity/schema corpus, полный native Rust suite и browser oracle. Browser job собирает WASM и viewer, выполняет два независимых полных capture, проверяет PNG/PDF repeatability, создаёт pixel-diff/JSON/HTML и classification, затем сохраняет оба actual run и отчёты как artifacts. Exit codes comparator не подавляются: visual/page mismatch завершает job с ошибкой, но report и classification сохраняются.

Workflow использует только сохранённые references и локальный Chromium/poppler. Office Online, LibreOffice и внешние renderer в нём не вызываются. После всех проверок отдельный `git diff --exit-code` подтверждает, что capture/diff не изменили `references` или `corpus.json`.

## Явное принятие baseline

Обычный capture/diff ничего не пишет в references. Кандидат создаётся только отдельной командой, с digest и двумя разными approvals; independent QA не может быть автором захвата:

```bash
node tools/strict-viewer/accept-baseline.mjs \
  --actual /tmp/strict-actual/docx/page-1/actual.png \
  --candidate test-fixtures/strict-viewer/candidates/docx/page-1.png \
  --approval approvals.json --accept-visual yes
```

Существующий файл не перезаписывается. После этого кандидат проходит baseline-only PR; команда сама не меняет corpus status и не делает reference accepted.
