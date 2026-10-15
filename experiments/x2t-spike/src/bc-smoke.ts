/**
 * Смоук пакета `@meerrec/browser-convert` на странице харнесса: конвертация через
 * `X2TConverter` и медиа-экстракция `extractDocxMediaUrls` из собранного dist пакета.
 *
 * Пакет подключён относительным путём — харнесс лежит вне pnpm-workspace, и имя пакета в нём
 * не резолвится (та же причина, что у `packages/x2t` в src/worker.ts и `@meerrec/core`
 * в src/viewer.ts). Отсюда и три `..`: файл живёт в `src/`, а `packages/` — в корне репозитория.
 * Сборка dist — забота вызывающего: `pnpm -F @meerrec/browser-convert build`; наличие
 * файла проверяет tools/preview-check.mjs перед прогоном смоука и просит собрать.
 *
 * Модуль в разметку не подключён: приёмка тянет его из evaluate динамическим import()
 * через Vite (`import('/src/bc-smoke.ts')`), поэтому код исполняется на главном потоке
 * страницы. Конвертация x2t блокирующая — на время вызова страница замирает; фикстуры мелкие,
 * и подъём модуля со страницы укладывается в бюджет из preview-check.mjs.
 *
 * ВАЖНО про область видимости: glue-скрипт x2t кладёт единственный `var Module` в область,
 * где исполняется. Воркер харнесса живёт в своей области — его модуль со страницей не
 * пересекается. Но на самой странице второй init невозможен (`loadX2t` бросает `X2tError`
 * поверх уже загруженного модуля), поэтому конвертер здесь один на страницу и
 * переиспользуется между вызовами смоука: повторный вызов init не делает вовсе — второй
 * прогон смоука это известный исход (`boot: 'reused'`), а не сбой.
 */
import { X2TConverter, extractDocxMediaUrls } from '../../../packages/browser-convert/dist/index.js';

/**
 * Артефакты те же, что у воркера харнесса: publicDir раздаёт их файлами в корне — отсюда
 * `/x2t.js`, а не `/x2t/x2t.js` (см. vite.config.ts).
 */
const SCRIPT_URL = '/x2t.js';
const WASM_URL = '/x2t.wasm';

/** Вход конвертации смоука: байты и имя, по которому конвертер выводит формат входа. */
export interface BcSmokeConvertInput {
  /** Байты входного файла (`.docx` и др.). */
  readonly bytes: Uint8Array;
  /** Имя файла с расширением: расширение задаёт формат входа, база имени — имя выхода. */
  readonly name: string;
  /** Целевой формат: расширение без точки, регистр не важен. */
  readonly targetExt: string;
}

/** Выжимка результата конвертации — простые поля, чтобы отчёт вернулся из evaluate. */
export interface BcSmokeConvertReport {
  /** Код возврата main1: 0 — успех; 80 — цель без движка раскладки (ADR-0024); 88 — кросс-семейная. */
  readonly code: number;
  /** Всё, что рантайм напечатал в stderr за вызов. */
  readonly stderr: string;
  /** Байты выхода; при ненулевом коде обычно пусто. */
  readonly outBytes: Uint8Array;
}

/** Результат медиа-экстракции. */
export interface BcSmokeMediaReport {
  /** Ключи медиа — пути вида `media/<имя>` без префикса `word/`; object URLs уже ревокнуты. */
  readonly mediaKeys: string[];
}

/** Что прогнать смоуку за один вызов: конвертация, медиа-экстракция или и то и другое. */
export interface BcSmokeInput {
  readonly convert?: BcSmokeConvertInput;
  /** Байты OOXML-контейнера (`.docx` и др.), из которого извлекать медиа. */
  readonly media?: Uint8Array;
}

/** Сводка одного вызова смоука — сериализуемая структура, страница возвращает её из evaluate. */
export interface BcSmokeReport {
  /**
   * Как поднят модуль x2t на странице: `booted` — init сделан этим вызовом, `reused` —
   * конвертер уже жил на странице (смоук гонялся), init повторно не звался, `null` —
   * конвертация не запрашивалась.
   */
  readonly boot: 'booted' | 'reused' | null;
  readonly convert: BcSmokeConvertReport | null;
  readonly media: BcSmokeMediaReport | null;
}

/** Один конвертер на страницу; промис — чтобы параллельные вызовы делили один подъём. */
let converterPromise: Promise<X2TConverter> | null = null;

/**
 * Поднимает (или отдаёт уже поднятый) конвертер страницы. Повторный init в той же области
 * невозможен — glue-скрипт кладёт единственный `Module` на глобальную область (см. шапку
 * модуля), поэтому инстанс переиспользуется. Отказ подъёма не кэшируем: следующий вызов
 * попробует снова — тот же расчёт, что у `ensureBoot` воркера харнесса.
 */
async function ensureConverter(): Promise<{ boot: 'booted' | 'reused'; converter: X2TConverter }> {
  const boot = converterPromise === null ? 'booted' : 'reused';
  if (converterPromise === null) {
    const converter = new X2TConverter();
    converterPromise = converter
      .init({ scriptUrl: SCRIPT_URL, wasmUrl: WASM_URL })
      .then(() => converter)
      .catch((error: unknown) => {
        converterPromise = null;
        throw error;
      });
  }
  const converter = await converterPromise;
  return { boot, converter };
}

/**
 * Смоук browser-convert на странице: конвертация через `X2TConverter` и ключи медиа через
 * `extractDocxMediaUrls`. Вход — байты `Uint8Array`, отчёт — простые сериализуемые структуры,
 * которые страница отдаёт из evaluate.
 *
 * @param input — что прогнать: конвертация (`convert`), медиа-экстракция (`media`) или обе.
 */
export async function bcSmoke(input: BcSmokeInput): Promise<BcSmokeReport> {
  let boot: BcSmokeReport['boot'] = null;
  let convert: BcSmokeConvertReport | null = null;
  let media: BcSmokeMediaReport | null = null;
  if (input.media !== undefined) {
    const urls = await extractDocxMediaUrls(input.media);
    const mediaKeys = Object.keys(urls);
    // Смоуку нужны только ключи; object URLs ревокаем, как обязывает API пакета.
    for (const url of Object.values(urls)) {
      URL.revokeObjectURL(url);
    }
    media = { mediaKeys };
  }
  if (input.convert !== undefined) {
    const ensured = await ensureConverter();
    boot = ensured.boot;
    const { bytes, name, targetExt } = input.convert;
    // Blob + явное имя: расширение имени задаёт формат входа, база — имя выхода.
    // Сужение типа без копии: байты пришли из сериализации evaluate и лежат на ArrayBuffer.
    const result = await ensured.converter.convert(new Blob([bytes as BlobPart]), targetExt, {
      fileName: name,
    });
    convert = { code: result.code, stderr: result.stderr, outBytes: result.bytes };
  }
  return { boot, convert, media };
}
