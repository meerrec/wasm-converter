/** Виды фикстур: у каждого свой корень в `/fixtures`. */
export type FixtureKind = 'docx' | 'xlsx';

/**
 * Прочитать фикстуру как байты — их принимает `viewer.open`.
 *
 * @param name — имя (для XLSX) или путь с категорией (для DOCX), как их отдаёт
 * `listFixtures`; путь собирается под корень вида.
 */
export async function fetchFixture(kind: FixtureKind, name: string): Promise<ArrayBuffer> {
  const response = await fetch(`${import.meta.env.BASE_URL}fixtures/${kind}/${name}`);
  if (!response.ok) throw new Error(`фикстура не найдена: ${name}`);
  return response.arrayBuffer();
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
