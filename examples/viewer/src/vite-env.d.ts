/// <reference types="vite/client" />

/**
 * Кастомных переменных окружения у примера больше нет: сайт редактора с его
 * `VITE_EUROOFFICE_BASE` уехал в отдельный репозиторий
 * (github.com/meerrec/eurooffice-demo, ADR-0038). Файл оставлен на случай новых
 * переменных: `vite/client` объявляет `ImportMetaEnv` с индексной подписью,
 * поэтому незадекларированное поле пришло бы как `any` — под `strict` это тихо,
 * а ошибку в имени переменной прячет.
 */
interface ImportMetaEnv {}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
