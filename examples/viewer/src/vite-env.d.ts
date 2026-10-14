/// <reference types="vite/client" />

/**
 * Переменные окружения примера.
 *
 * `vite/client` объявляет `ImportMetaEnv` с индексной подписью, поэтому
 * незадекларированное поле пришло бы как `any` — под `strict` это тихо, а
 * ошибку в имени переменной прячет. Явное поле возвращает `string | undefined`
 * и подсвечивает опечатку в `import.meta.env`.
 */
interface ImportMetaEnv {
  /**
   * Адрес хоста сайта редактора Euro-Office в дев-режиме (`node
   * scripts/eurooffice/serve.mjs`). Задан в `.env.development`, который Vite
   * читает только в режиме разработки: в прод-сборке значение не определено.
   */
  readonly VITE_EUROOFFICE_BASE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
