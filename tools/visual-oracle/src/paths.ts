// Пути по умолчанию привязаны к корню репозитория, а не к текущему каталогу:
// команда запускается и из корня, и из каталога пакета, и из `dist/`, а схема
// manifest и корень фикстур лежат в одном месте для всех случаев.

import { dirname, isAbsolute, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * Корень репозитория.
 *
 * Модуль живёт в `tools/visual-oracle/src` (или `dist` после сборки), поэтому
 * корень — три уровня вверх в обоих случаях.
 */
export const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');

/**
 * Разрешить путь относительно корня репозитория, если он не абсолютный.
 *
 * `resolve` нормализует и абсолютные пути: хвостовой `/` и `..` исчезают, а
 * сравнение сегментов пути (например, `<format>/<fixture-id>/<mode>`) не
 * должно зависеть от того, как оператор набрал каталог.
 */
export function resolveFromRepo(path: string): string {
  return isAbsolute(path) ? resolve(path) : resolve(REPO_ROOT, path);
}
