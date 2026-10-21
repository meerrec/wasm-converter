# Архитектурный аудит `meerrec/wasm-converter` (doc-converter)

Документ описывает:

1. **As-Is** — как проект устроен сейчас.
2. **Долги и риски** — что мешает развитию.
3. **To-Be** — целевая архитектура.
4. **Матрицу сравнения** — As-Is vs To-Be.
5. **Roadmap миграции** — по фазам.
6. **Критические замечания архитектора**.

> **Статус на 21.10.2026:** Фаза 1 реализована — ADR-0027 (npmjs как основной реестр), ADR-0028 (wire-типы из Rust через ts-rs), ADR-0029 (main-thread backend без `OffscreenCanvas`). Разделы As-Is и To-Be ниже — снимок на момент аудита, а не текущее состояние.

---

## Фаза 1. As-Is: текущая архитектура

### 1.1. Сборка и дистрибуция

- **Монорепозиторий**:
  - JS/TS: `pnpm workspace` + `turbo`
  - Rust: `Cargo workspace`
- **Rust-крейты**:
  - `core` — общие типы и логика
  - `render` — векторный/растровый пайплайн
  - `xlsx` — таблицы
  - `docx` — документы
  - `pdf` — собственный PDF-генератор
  - 3 WASM-точки входа: `doc_converter_wasm`, `doc_converter_docx_wasm`, `doc_converter_pdf_wasm`
- **JS/TS-пакеты**:
  - `@meerrec/core`
  - `@meerrec/wasm`
  - `@meerrec/wasm-docx`
  - `@meerrec/wasm-pdf`
  - `@meerrec/cli`
- **Публикация**: GitHub Packages (`npm.pkg.github.com`, scope `@meerrec`).
- **npmjs.org**: не используется.
- **crates.io**: non-goal (ADR-0026).
- **Версионирование**: `@changesets/cli`, текущая версия `v1.0.0`.
- **Размеры (gzip)**:
  - XLSX WASM: ~464 КБ
  - DOCX WASM: ~384 КБ
  - PDF WASM: ~1,01 МБ (из-за шрифта Carlito)

### 1.2. Rust/WASM-ядро

Слои:

- **Парсинг**: OOXML XML через `quick-xml`, ZIP-архивы.
- **Модель**: внутреннее представление документа/таблицы.
- **Раскладка**: геометрия страниц (DOCX), ячеек/сеток (XLSX).
- **Рендеринг**: команды для `OffscreenCanvas` через Canvas2D.
- **Экспорт**: PDF, Markdown, HTML, Plain Text.

Особенности:

- кэш метрик шрифтов `FontRegistry`;
- обработка переполнения ячеек `#####`;
- постраничный SSIM-анализ.

### 1.3. Транспорт данных

Схема: **Main Thread ↔ Web Worker (WASM)**.

Режимы:

1. **`sab` (SharedArrayBuffer)** — ring-буфер глубиной 2, zero-copy, frame dropping.
2. **`bytes` (ArrayBuffer Transfer)** — двойная буферизация с передачей владения.

Режим `auto`:

- есть cross-origin isolation → `sab`;
- иначе → `bytes`.

Задержки RPC:

- SAB: < 1 мс
- bytes: < 3 мс

```text
[ Main Thread (Viewer API) ]
      │
      ├── (Command: Open/Render/Zoom) ──> [ Web Worker (Rust/WASM Engine) ]
      │                                            │
[ Canvas Element ] <── (OffscreenCanvas / SAB) ────┘
```

### 1.4. COOP/COEP и Service Worker

Для `SharedArrayBuffer` нужны заголовки:

- `Cross-Origin-Opener-Policy: same-origin`
- `Cross-Origin-Embedder-Policy: require-corp`

На GitHub Pages используется `sw.js`, который динамически подставляет заголовки для Chromium.

Ограничения:

- Firefox и Safari на GitHub Pages работают через `bytes` без изоляции.

### 1.5. Публичный API

- Фабрики:
  - `createXlsxViewer(host, config)`
  - `createDocxViewer(host, config)`
- Методы:
  - `open(bytes)`
  - `showPage(n)`
  - `showSheet(n)`
  - `setZoom(z)`
  - `exportPdf()`
  - `exportPng()`
  - `destroy()`
- DOM-зависимость минимальная: рендер изолирован в Web Worker через `OffscreenCanvas`.

### 1.6. CLI и SDK

- CLI: `@meerrec/cli` на Node.js, `worker_threads`, команды `convert` и `batch`.
- SDK для Python/C++/Go/Java отсутствуют.

### 1.7. Производительность

| Метрика | As-Is | Бюджет |
|---|---:|---:|
| DOCX Parse (50 МБ) | 356 мс | < 1.5 с |
| DOCX Layout (150 стр.) | 32.47 мс | < 500 мс |
| XLSX PDF Export (500 стр.) | 159.4 мс | < 3 с |
| Пик памяти PDF Export 500 стр. | 28.12 МиБ | ≤ 40 МиБ |
| Пик памяти DOCX 57 МБ, 75 картинок | 155.50 МиБ | ≤ 200 МиБ |
| SSIM DOCX | 0.9925 | LibreOffice Oracle |
| SSIM XLSX | 0.9844 | LibreOffice Oracle |

### 1.8. Надёжность и тесты

- e2e: Playwright/Puppeteer.
- Визуальные регрессии: SSIM против LibreOffice.
- Покрытие Rust: `docx ≥ 85%`, `core ≥ 80%`.
- Fuzzing/property-based: явно не описаны.

### 1.9. Ограничения v1.0.0

- Редактирование — non-goal.
- Сноски/комментарии парсятся, но игнорируются.
- Нет обтекания текста вокруг плавающих объектов.
- VML пропускается.
- Не поддерживаются: Strict OOXML, PPTX, XLSB, `.docm`, зашифрованные и бинарные `.doc`.

---

## Фаза 2. Архитектурные долги

| Долг | Критичность | Суть |
|---|---|---|
| GitHub Packages требует PAT | Высокая | Блокирует внешнее adoption |
| 3 отдельных WASM | Средняя | Дублирование std и структур |
| Нет fallback без OffscreenCanvas | Средняя | Не работает в старых Safari/WebView |
| Монолитный Worker IPC | Средняя | Ручной `postMessage`, риск рассинхрона |
| ZIP читается целиком в RAM | Высокая | OOM и zip bomb |
| AGPL-демо Euro-Office | Низкая | Юридический риск для enterprise |

---

## Фаза 3. To-Be: целевая архитектура

### 3.1. Схема

```text
┌─────────────────────────────────────────────────────────────────────────────┐
│                             MAIN THREAD (CLIENT)                            │
│  ┌───────────────────────┐  ┌─────────────────────┐  ┌───────────────────┐  │
│  │ React / Vue / Svelte  │  │ Core Engine SDK     │  │ WebGL / Canvas    │  │
│  └───────────┬───────────┘  └──────────┬──────────┘  └─────────▲─────────┘  │
└──────────────┼─────────────────────────┼───────────────────────┼────────────┘
              │ Async RPC               │ SharedArrayBuffer     │ Paint Cmds
              ▼                         ▼                       │
┌────────────────────────────────────────────────────────────────┴────────────┐
│                             WEB WORKER RUNTIME                              │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                        Document Engine Router                         │  │
│  └───────┬──────────────────────────────┬─────────────────────────┬──────┘  │
│          ▼                              ▼                         ▼         │
│  ┌───────────────┐              ┌───────────────┐         ┌──────────────┐  │
│  │ FormatPlugin  │              │ FormatPlugin  │         │ FormatPlugin │  │
│  │    (DOCX)     │              │    (XLSX)     │         │ (PPTX/PDF)   │  │
│  └───────┬───────┘              └───────┬───────┘         └──────┬───────┘  │
│          └──────────────────────┬───────┘                        │          │
│                                 ▼                                │          │
│                      ┌─────────────────────┐                     │          │
│                      │ Layout & Paint IR   │<────────────────────┘          │
│                      └──────────┬──────────┘                                │
│                                 ▼                                           │
│                      ┌─────────────────────┐                                │
│                      │ Raster/Vector Engine│                               │
│                      └─────────────────────┘                                │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 3.2. Rust-трейты

```rust
use async_trait::async_trait;
use std::io::{Read, Seek};

pub struct RenderContext {
    pub scale: f32,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

pub trait RenderCommand {
    fn execute(&self, canvas: &mut dyn CanvasPainter);
}

pub trait CanvasPainter {
    fn draw_text(&mut self, text: &str, x: f32, y: f32, font: &str, size: f32);
    fn draw_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32);
    fn draw_image(&mut self, image_bytes: &[u8], x: f32, y: f32, w: f32, h: f32);
}

#[async_trait]
pub trait FormatPlugin: Send + Sync {
    fn id(&self) -> &'static str;
    fn can_parse(&self, header: &[u8]) -> bool;

    async fn parse_stream(
        &self,
        reader: Box<dyn ChunkReader>,
    ) -> Result<Box<dyn DocumentModel>, EngineError>;
}

pub trait DocumentModel: Send + Sync {
    fn page_count(&self) -> u32;
    fn compute_layout(&mut self, ctx: &RenderContext) -> Vec<Box<dyn RenderCommand>>;
    fn export_pdf(&self) -> Result<Vec<u8>, EngineError>;
}

pub trait ChunkReader: Read + Seek + Send + Sync {
    fn total_size(&self) -> u64;
}
```

### 3.3. TypeScript API

```ts
export interface ViewerConfig {
  canvas: HTMLCanvasElement;
  transport?: 'auto' | 'sab' | 'bytes';
  fallbackToMainThread?: boolean;
}

export class DocConverterEngine {
  private worker: Worker;

  static async create(config: ViewerConfig): Promise<DocConverterEngine> {
    const engine = new DocConverterEngine(config);
    await engine.init();
    return engine;
  }

  async loadDocument(source: ArrayBuffer | ReadableStream<Uint8Array>): Promise<void> {
    // инкрементальная передача чанков воркеру
  }

  async renderPage(pageIndex: number, scale: number): Promise<void> {
    // команда на рендер
  }

  async destroy(): Promise<void> {
    this.worker.terminate();
  }
}
```

---

## Фаза 4. Матрица As-Is vs To-Be

| Аспект | As-Is | To-Be | Выигрыш |
|---|---|---|---|
| Реестр | GitHub Packages (PAT) | npmjs.org + CDN | Нулевой барьер установки |
| Форматы | DOCX, XLSX | DOCX, XLSX, PPTX, ODF | Офисный стек |
| WASM | 3 бинарника | Единое ядро + плагины | −25% размера |
| Парсинг | Файл целиком в RAM | Chunked Streaming | −60% пика памяти |
| Canvas | OffscreenCanvas | + Main Thread Fallback | 100% браузеров |
| Рендер | Canvas2D | Canvas2D + WebGL | 60 FPS на сложных таблицах |
| Оракулы | LibreOffice SSIM | + MS Office SSIM | Точность вёрстки |
| UI | Ванильный JS | React/Vue/Svelte | DX |
| Наблюдаемость | Локальные callbacks | OpenTelemetry | APM |
| CLI | Node.js | Node, Deno, Bun | Кросс-рантайм |
| Тесты | unit + e2e | + cargo-fuzz | Устойчивость к битым ZIP/XML |
| OOXML | Transitional | + Strict | Совместимость |
| IPC | Ручной postMessage | Typed RPC | Типобезопасность |
| Лицензии | Apache + AGPL в демо | Изоляция AGPL | Enterprise-safe |
| Изоляция | SW-патчинг | SW + native COOP/COEP | Устойчивый SAB |

---

## Фаза 5. Roadmap миграции

### Фаза 1. Быстрые победы (1–2 спринта)

- Публикация в `npmjs.org`.
- Typed RPC через `ts-rs`.
- Main Thread Canvas Fallback.

**Результат:** установка без токена, защита IPC, поддержка старых браузеров.

### Фаза 2. Архитектурные улучшения (2–4 спринта)

- Trait-based ядро: `FormatPlugin`, `CanvasPainter`.
- Единое динамическое загрузчик WASM.
- Инкрементальный потоковый ZIP.

**Результат:** пик RAM < 60 МиБ, меньше дублирования.

### Фаза 3. Расширение форматов (3–6 спринтов)

- PPTX.
- Strict OOXML.
- WebGL для XLSX > 100 000 ячеек.

**Результат:** презентации и плотные таблицы.

### Фаза 4. Индустриализация (2–4 спринта)

- React, Vue, Svelte обёртки.
- `cargo-fuzz` для parser-слоя.
- CLI под Deno и Bun.

**Результат:** готовность к Enterprise.

---

## Фаза 6. Критические замечания архитектора

1. **GitHub Packages — блокер.** Срочно переносить на npmjs.org.
2. **OOM и zip bomb.** Нужен потоковый ZIP-парсер с лимитом памяти.
3. **Service Worker для COOP/COEP хрупок.** Во вложенных iframe может не работать. Нужен прозрачный fallback на `bytes`.
4. **Нет отмены операций.** Тяжёлая раскладка блокирует воркер. Нужен `AbortSignal` в Rust и проверка статуса между страницами.
5. **AGPL-соседство.** Euro-Office под AGPL-3.0 лучше вынести в отдельный репозиторий.

---

## Итог

- **Сейчас:** быстрый, но узкий и закрытый конвертер DOCX/XLSX в браузере.
- **Цель:** платформенный SDK с плагинами форматов, потоковым парсингом, единым WASM, fallback-рендерингом, публикацией в npm, typed RPC и чистой лицензионной моделью.
- **Приоритеты:** npm-дистрибуция, потоковый ZIP, fallback Canvas, typed RPC, вынос AGPL.
