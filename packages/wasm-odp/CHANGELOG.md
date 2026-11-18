# @meerrec/wasm-odp

## 2.2.0

### Patch Changes

- Исправления после 2.1.0: висящие hit-test/hyperlink при destroy и ошибке
  воркера, пробелы в текстовых XLSX, парные теги нумерации DOCX, выравнивание
  DisplayList; xlsx больше не зависит от chrono.

## 2.1.0

### Patch Changes

- ccac422: Четыре ленивых wasm-модуля (XLSB, ODS, ODT, ODP) и фабрики
  `createXlsbViewer`/`createOdsViewer`/`createOdtViewer`/`createOdpViewer`:
  воркер на формат, модуль грузится по URL при открытии файла.
  CLI понимает `.xlsb`, `.ods`, `.odt`, `.odp` на входе.

## 2.0.2

### Patch Changes
