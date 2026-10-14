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
 * Сколько ждать активации воркера, прежде чем смириться с транспортом `bytes`.
 *
 * Ожидание не вечное: воркер, который не встал (404 на `sw.js`, обрыв сети),
 * не должен задерживать инициализацию приложения дольше, чем стоит попытка
 * получить SAB. По этому таймауту функция завершается без перезагрузки: без
 * активного воркера она всё равно ничего не изменит. Страница без изоляции
 * работает и так (ADR-0021).
 */
const ACTIVATION_TIMEOUT_MS = 5000;

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

/** Встал ли у регистрации воркер, готовый перехватить следующую навигацию. */
function isActivated(registration: ServiceWorkerRegistration): boolean {
  return registration.active?.state === 'activated';
}

/**
 * Дождаться активации воркера регистрации, но не дольше `timeoutMs`;
 * `false` — воркер так и не встал.
 *
 * `register()` резолвится сразу после создания регистрации, а не установки:
 * перезагрузка, попавшая в навигацию до активации, до воркера не дошла бы — и
 * страница осталась бы неизолированной, уже израсходовав единственную попытку
 * (метку в `sessionStorage`). Поэтому ждём именно состояния `activated`, при
 * котором воркер перехватывает навигации.
 */
function waitForActivation(
  registration: ServiceWorkerRegistration,
  timeoutMs: number,
): Promise<boolean> {
  if (isActivated(registration)) return Promise.resolve(true);

  return new Promise<boolean>((resolve) => {
    // Воркер по ходу установки переезжает между слотами регистрации
    // (`installing` → `waiting` → `active`), поэтому слушаем тот, в котором он
    // сейчас; смена состояния видна на нём же до самого конца.
    const worker = registration.installing ?? registration.waiting ?? registration.active;

    function finish(): void {
      worker?.removeEventListener('statechange', onStateChange);
      window.clearTimeout(timer);
      resolve(isActivated(registration));
    }

    function onStateChange(): void {
      // `redundant` — воркер не встанет никогда: ждать нечего.
      if (isActivated(registration) || worker?.state === 'redundant') finish();
    }

    const timer = window.setTimeout(finish, timeoutMs);
    worker?.addEventListener('statechange', onStateChange);
  });
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

  // Ждём активации: перезагрузка, стартовавшая раньше, попала бы в навигацию, до
  // которой воркер не дорос, и изоляция не появилась бы вовсе. Воркер не встал за
  // отведённое время — перезагружаться нечем: заголовков у новой навигации не
  // будет, а метка единственной попытки сгорит зря.
  if (!(await waitForActivation(registration, ACTIVATION_TIMEOUT_MS))) return;

  // Признак один — изоляции у страницы так и нет. Текущий документ ушёл браузеру
  // задолго до регистрации, а `crossOriginIsolated` у загруженного документа сам
  // не включается: COOP/COEP приходят с ответом. Изоляцию получает только
  // документ, чей ответ прошёл через воркер, — значит, лечится это лишь
  // следующей навигацией, то есть перезагрузкой.
  //
  // Раньше здесь стояло `navigator.serviceWorker.controller === null`, и его не
  // хватало: воркер на `activate` зовёт `clients.claim()`, который выставляет
  // контроллер текущему документу немедленно. Условие становилось ложным, хотя
  // изолированным документ от этого не становился, — и первый заход в сессию ехал
  // на транспорте `bytes`, а SAB появлялся только со второго захода.
  if (window.crossOriginIsolated) return;
  if (alreadyReloaded || !markReloaded()) return;
  window.location.reload();
}
