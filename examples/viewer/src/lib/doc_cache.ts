// Кэш байтов в IndexedDB: исходных документов (фикстуры и файлы пользователя)
// и wasm-модулей (JS-обвязка и бинарник). Нужен он тому, что и так лежит
// рядом: второе открытие фикстуры и повторный запуск страницы не должны платить
// сетью за те же байты.
//
// Кэш — ускорение, а не условие работы. Без хранилища страница открывает
// документы и экспортирует PDF ровно так же, только каждый раз по сети;
// поэтому любая ошибка IndexedDB (приватный режим, запрет хранилища политикой
// браузера, отсутствие `indexedDB`, переполнение квоты) здесь глушится:
// чтение отдаёт `null`, запись — ничего не делает, а индикатор показывает `—`.

/** Имя базы: одно на приложение, чтобы `clearCache` чистил всё разом. */
const DB_NAME = 'doc-converter-viewer';

/** Версия схемы. Растёт, когда меняется набор хранилищ: `onupgradeneeded`. */
const DB_VERSION = 1;

/** Хранилище документов. Ключ — строка: `fixture/…` или `file/…`. */
const DOCUMENTS_STORE = 'documents';

/** Хранилище wasm-модулей. Ключ — URL файла, ровно тот, что уходит в `fetch`. */
const WASM_STORE = 'wasm';

/**
 * Соединение открывается лениво — при первом обращении, — и живёт в модуле:
 * страница обращается к базе и при открытии документа, и при экспорте, а
 * каждое открытие базы стоит отдельной транзакции движка.
 */
let dbPromise: Promise<IDBDatabase> | null = null;

/** Сообщение для `null`-случая: причина отказа наружу не выносится. */
const UNAVAILABLE = 'IndexedDB недоступен';

/** Открыть базу (или взять уже открытую). Отказ — исключение, его глушат вызовы. */
function openDb(): Promise<IDBDatabase> {
  dbPromise ??= new Promise<IDBDatabase>((resolve, reject) => {
    if (typeof indexedDB === 'undefined') {
      reject(new Error(UNAVAILABLE));
      return;
    }
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    // Схема описывается только здесь: обработчик вызывается, когда базы нет
    // или её версия меньше объявленной, — ветвиться по версиям в вызовах
    // больше негде.
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(DOCUMENTS_STORE)) db.createObjectStore(DOCUMENTS_STORE);
      if (!db.objectStoreNames.contains(WASM_STORE)) db.createObjectStore(WASM_STORE);
    };
    request.onsuccess = () => {
      resolve(request.result);
    };
    request.onerror = () => {
      reject(request.error ?? new Error('IndexedDB: открытие базы не удалось'));
    };
    // Базу держит открытой другая вкладка со старой версией схемы: ждать её
    // бессмысленно, а страница обязана остаться рабочей.
    request.onblocked = () => {
      reject(new Error('IndexedDB: база занята другой вкладкой'));
    };
  });
  // Неудачу не запоминаем: следующий вызов попробует снова. Иначе один отказ
  // (в приватном окне он может случиться на первом же открытии) выключил бы
  // кэш до перезагрузки страницы.
  dbPromise.catch(() => {
    dbPromise = null;
  });
  return dbPromise;
}

/** Прочитать значение хранилища; `null` — ключа нет, значения нет или отказ. */
async function readBytes(store: string, key: string): Promise<ArrayBuffer | null> {
  try {
    const db = await openDb();
    return await new Promise<ArrayBuffer | null>((resolve, reject) => {
      const request = db.transaction(store, 'readonly').objectStore(store).get(key);
      request.onsuccess = () => {
        const value: unknown = request.result;
        // В базе лежит только то, что мы туда положили, но ответ IndexedDB для
        // TypeScript — `unknown`: доверяем лишь `ArrayBuffer`.
        resolve(value instanceof ArrayBuffer ? value : null);
      };
      request.onerror = () => {
        reject(request.error ?? new Error('IndexedDB: чтение не удалось'));
      };
    });
  } catch {
    return null;
  }
}

/** Записать значение в хранилище. Отказ записи — молчание: это только кэш. */
async function writeBytes(store: string, key: string, bytes: ArrayBuffer): Promise<void> {
  try {
    const db = await openDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(store, 'readwrite');
      tx.objectStore(store).put(bytes, key);
      // Ждём завершения транзакции, а не ответа на запрос: байты попадают в
      // базу на коммите, и сорвавшаяся транзакция (квота, диск) должна
      // всплыть здесь, а не потеряться.
      tx.oncomplete = () => {
        resolve();
      };
      tx.onerror = () => {
        reject(tx.error ?? new Error('IndexedDB: запись не удалась'));
      };
      tx.onabort = () => {
        reject(tx.error ?? new Error('IndexedDB: транзакция отменена'));
      };
    });
  } catch {
    // Переполнение квоты и прочие отказы: страница работает и без кэша.
  }
}

/**
 * Байты документа из кэша.
 *
 * @param key — ключ документа: `fixture/<kind>/<name>` для фикстуры или
 * `file/<name>/<size>/<lastModified>` для файла пользователя.
 * @returns байты или `null`, если документа в кэше нет либо хранилище недоступно.
 */
export async function getDocument(key: string): Promise<ArrayBuffer | null> {
  return readBytes(DOCUMENTS_STORE, key);
}

/**
 * Положить байты документа в кэш. Ошибка записи не всплывает: кэш — ускорение.
 *
 * @param key — тот же ключ, что и у `getDocument`.
 * @param bytes — байты; в базе оказывается копия, исходный буфер можно отдавать
 * дальше (в том числе передавать в воркер).
 */
export async function putDocument(key: string, bytes: ArrayBuffer): Promise<void> {
  await writeBytes(DOCUMENTS_STORE, key, bytes);
}

/**
 * Ключ файла пользователя в кэше документов: имя, размер и время правки.
 *
 * Имя у разных файлов совпадает, поэтому одного имени мало; размер с временем
 * правки отличают и версии одного файла. Содержимое при этом не читается —
 * ключ строится до загрузки байтов.
 *
 * @param file — выбранный или перетащенный файл.
 */
export function fileDocumentKey(file: File): string {
  return `file/${file.name}/${file.size}/${file.lastModified}`;
}

/**
 * Байты wasm-модуля из кэша.
 *
 * @param url — URL файла модуля (JS-обвязки или бинарника), как он уходит в `fetch`.
 * @returns байты или `null`, если модуля в кэше нет либо хранилище недоступно.
 */
export async function getWasm(url: string): Promise<ArrayBuffer | null> {
  return readBytes(WASM_STORE, url);
}

/**
 * Положить байты wasm-модуля в кэш. Ошибка записи не всплывает: кэш — ускорение.
 *
 * @param url — тот же URL, что и у `getWasm`: он и есть ключ.
 * @param bytes — байты модуля.
 */
export async function putWasm(url: string, bytes: ArrayBuffer): Promise<void> {
  await writeBytes(WASM_STORE, url, bytes);
}

/** Сколько документов и сколько байт wasm лежит в кэше. */
export interface CacheStats {
  /** Число документов в хранилище `documents`. */
  docs: number;
  /** Суммарный размер wasm-модулей в байтах. */
  wasmBytes: number;
}

/** Число записей хранилища. */
async function countRecords(store: string): Promise<number> {
  const db = await openDb();
  return await new Promise<number>((resolve, reject) => {
    const request = db.transaction(store, 'readonly').objectStore(store).count();
    request.onsuccess = () => {
      resolve(request.result);
    };
    request.onerror = () => {
      reject(request.error ?? new Error('IndexedDB: подсчёт не удался'));
    };
  });
}

/** Суммарный размер записей хранилища в байтах. */
async function sumBytes(store: string): Promise<number> {
  const db = await openDb();
  return await new Promise<number>((resolve, reject) => {
    const request = db.transaction(store, 'readonly').objectStore(store).openCursor();
    let total = 0;
    request.onsuccess = () => {
      const cursor = request.result;
      if (!cursor) {
        resolve(total);
        return;
      }
      const value: unknown = cursor.value;
      if (value instanceof ArrayBuffer) total += value.byteLength;
      cursor.continue();
    };
    request.onerror = () => {
      reject(request.error ?? new Error('IndexedDB: обход не удался'));
    };
  });
}

/**
 * Мерки кэша — для индикатора в строке состояния.
 *
 * @returns число документов и суммарный размер wasm; `null` — хранилище
 * недоступно, и индикатор показывает `—`. Отличать «пусто» от «недоступно»
 * обязательно: `0` документов — обычное состояние после первой очистки, и
 * путать его с отказом хранилища значило бы врать в строке состояния.
 */
export async function cacheStats(): Promise<CacheStats | null> {
  try {
    const [docs, wasmBytes] = await Promise.all([
      countRecords(DOCUMENTS_STORE),
      sumBytes(WASM_STORE),
    ]);
    return { docs, wasmBytes };
  } catch {
    return null;
  }
}

/**
 * Очистить оба хранилища. Нужна тестам и ручной проверке: кэш невидим, и без
 * способа его сбросить застрявшие байты нечем было бы выкинуть.
 *
 * Ошибка очистки не всплывает: как и остальные отказы хранилища, она не повод
 * ронять страницу.
 */
export async function clearCache(): Promise<void> {
  try {
    const db = await openDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction([DOCUMENTS_STORE, WASM_STORE], 'readwrite');
      tx.objectStore(DOCUMENTS_STORE).clear();
      tx.objectStore(WASM_STORE).clear();
      tx.oncomplete = () => {
        resolve();
      };
      tx.onerror = () => {
        reject(tx.error ?? new Error('IndexedDB: очистка не удалась'));
      };
      tx.onabort = () => {
        reject(tx.error ?? new Error('IndexedDB: очистка отменена'));
      };
    });
  } catch {
    // См. шапку модуля: хранилище — ускорение, а не условие работы.
  }
}
