// Кросс-origin изоляция страницы: COOP/COEP открывают доступ к `SharedArrayBuffer`,
// через который вьюер гоняет кадры в воркер. Без изоляции он уходит на транспорт
// `bytes` (ADR-0021) — медленнее, но работает.

import { euroofficeBaseUrl } from './viewer_urls.js';

/**
 * Метка в `sessionStorage`: страница уже перезагружалась ради изоляции.
 *
 * Заголовки приходят с ответом, поэтому перезагрузка — единственный способ их
 * получить. Без метки она бы зациклилась: если воркер заголовки не добавил,
 * отсутствие изоляции от перезагрузки само по себе не изменится.
 */
const RELOAD_FLAG = 'coiReloadedBySelf';

/**
 * Прочитать и снять метку перезагрузки.
 *
 * Снимается, а не только читается: следующая навигация в этой вкладке получает
 * новую попытку, а немедленная перезагрузка — уже нет. `true` здесь означает
 * «перезагружаться нельзя»: в приватном режиме хранилища нет, а метку поставить
 * негде — цикла тогда не избежать.
 */
function takeReloadMark(): boolean {
  try {
    const seen = window.sessionStorage.getItem(RELOAD_FLAG) !== null;
    window.sessionStorage.removeItem(RELOAD_FLAG);
    return seen;
  } catch {
    return true;
  }
}

/** Поставить метку перед перезагрузкой; `false` — места для метки нет. */
function markReloaded(): boolean {
  try {
    window.sessionStorage.setItem(RELOAD_FLAG, '1');
    return true;
  } catch {
    return false;
  }
}

/**
 * Умеет ли браузер встроить кадр в изолированную страницу.
 *
 * Источник правды для всех, кому нужен этот ответ: корневой воркер
 * (`scripts/pages/sw.js`) и inline-скрипт `scripts/pages/index.html` повторяют ту
 * же формулировку, потому что воркер изолирует страницу на следующем заходе, а
 * страницы решают, поднимать ли воркер вовсе. Разъедутся — маршрут `#/eurooffice`
 * умрёт молча: вкладка выглядит живой, а рамка не грузится.
 *
 * Изоляция сайта держится на `COEP: credentialless` у документов движка: без
 * `credentialless` у движка режутся воркер-скрипты (`spell.js`, `x2t.worker.js`),
 * и проверка орфографии с конвертером таблиц отваливаются. Firefox и Safari
 * `credentialless` не знают и остаются без изоляции — вьюер едет на транспорте
 * `bytes` (ADR-0021), это штатный путь.
 *
 * Проверяется `userAgentData`, а не атрибут `credentialless` у
 * `HTMLIFrameElement`: в воркере DOM нет, а API клиент-хинтов экспонировано и в
 * Worker'ах, так что одна формулировка годится всем трём местам. Прокси точный —
 * браузера, знающего `credentialless`, но не `userAgentData`, сегодня нет: в
 * Chromium есть и то и другое, а Firefox и Safari не знают ни одного.
 */
export function supportsCredentiallessFrames(): boolean {
  return 'userAgentData' in navigator;
}

/**
 * Обеспечить странице кросс-origin изоляцию: зарегистрировать корневой service
 * worker сайта и, если изоляция так и не появилась, один раз перезагрузить
 * страницу.
 *
 * Заголовки ставит единственный воркер сайта (`/<repo>/sw.js`), а не своя копия
 * в `public/`: навигация iframe обслуживается воркером, контролирующим родителя,
 * поэтому воркер в области `/viewer/` навигацию `editor.html` в корне не увидел
 * бы, и редактор Euro-Office остался бы без COEP. В dev и `vite preview`
 * заголовки ставит Vite, и воркер там не нужен: `sw.js` по этому адресу нет, и
 * регистрация обернулась бы 404 в консоли.
 *
 * Функция ничего не возвращает и ни при каких отказах не бросает: изоляция —
 * ускорение, а не условие работы. Недоступный `serviceWorker`, отсутствие `sw.js`
 * по адресу, запрет регистрации — страница просто останется неизолированной,
 * как и было задумано штатным путём без SAB.
 */
export async function ensureCrossOriginIsolation(): Promise<void> {
  if (window.crossOriginIsolated) return;

  // Небезопасный контекст и приватный режим: `serviceWorker` просто нет.
  if (typeof navigator.serviceWorker === 'undefined') return;

  // Без поддержки изоляция превратила бы `#/eurooffice` в мёртвый маршрут (см.
  // `supportsCredentiallessFrames`), поэтому воркер тут не регистрируется —
  // молча, как и всё остальное в этой функции.
  if (!supportsCredentiallessFrames()) return;

  const alreadyReloaded = takeReloadMark();

  let registration: ServiceWorkerRegistration;
  try {
    registration = await navigator.serviceWorker.register(`${euroofficeBaseUrl}sw.js`);
  } catch {
    return;
  }

  // Текущий документ ушёл браузеру задолго до регистрации — заголовки принесёт
  // только следующая навигация, и начнёт контролироваться она же. Нет активного
  // воркера или страница уже контролируется — перезагружаться незачем.
  if (registration.active === null || navigator.serviceWorker.controller !== null) return;
  if (alreadyReloaded || !markReloaded()) return;
  window.location.reload();
}
