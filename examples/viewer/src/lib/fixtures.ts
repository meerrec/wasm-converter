import { getDocument, putDocument } from './doc_cache.js';

/** Виды фикстур: у каждого свой корень в `/fixtures`. */
export type FixtureKind = 'docx' | 'xlsx' | 'pptx';

/** Ключ фикстуры в кэше документов: вид и имя через те же разделители, что в пути. */
function fixtureKey(kind: FixtureKind, name: string): string {
  return `fixture/${kind}/${name}`;
}

/**
 * Прочитать фикстуру как байты — их принимает `viewer.open`.
 *
 * Первым делом заглядывает в кэш: фикстуры — те же файлы на диске, и второе
 * открытие той же фикстуры (в том числе после перезагрузки страницы) не должно
 * снова тянуть её по сети.
 *
 * @param name — имя (для XLSX) или путь с категорией (для DOCX), как их отдаёт
 * `listFixtures`; путь собирается под корень вида.
 */
export async function fetchFixture(kind: FixtureKind, name: string): Promise<ArrayBuffer> {
  const key = fixtureKey(kind, name);
  const cached = await getDocument(key);
  if (cached !== null) return cached;

  const response = await fetch(`${import.meta.env.BASE_URL}fixtures/${kind}/${name}`);
  // Отсутствие фикстуры — по-прежнему ошибка, а не пустой документ, и до записи
  // в кэш: класть туда нечего, а сообщение о 404 остаётся прежним.
  if (!response.ok) throw new Error(`фикстура не найдена: ${name}`);
  const bytes = await response.arrayBuffer();
  // Записи дожидаемся, пока байты ещё наши: их отдадут дальше, а вызывающий
  // вправе передать буфер воркеру (перенос обнулил бы его). Отказ записи глотает
  // `putDocument` — отсутствие кэша не превращает открытие документа в ошибку.
  await putDocument(key, bytes);
  return bytes;
}

/**
 * Список фикстур вида — для выпадающего списка. Список фикстур — удобство
 * разработки, без него пример работает: файл можно выбрать и перетащить.
 * Поэтому недоступный `index.json` — не ошибка, а пустой список.
 */
export async function listFixtures(kind: FixtureKind): Promise<string[]> {
  try {
    const response = await fetch(`${import.meta.env.BASE_URL}fixtures/${kind}/index.json`);
    if (!response.ok) return [];
    const names: unknown = await response.json();
    // Данные приходят из файла на диске: доверяем только строкам.
    return Array.isArray(names) ? names.filter((n): n is string => typeof n === 'string') : [];
  } catch {
    return [];
  }
}
