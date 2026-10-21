// Единственный сервис-воркер сайта, корневой. GitHub Pages не отдаёт заголовки
// cross-origin isolation (COOP/COEP), поэтому их ставит этот воркер: с ними у
// страницы демо поднимается SharedArrayBuffer и транспорт кадров `sab`
// (ADR-0021). Воркер ничего не кэширует: по корневому `/sw.js` раньше жил
// PWA-воркер прежней версии сайта (времен движка Euro-Office), и его кэши
// сносятся при активации, чтобы не отдавать посетителю устаревшие страницы.
//
// Регистрирует воркер страница демо (`examples/viewer/src/lib/isolation.ts`),
// только пока изоляции нет и если браузер умеет встраивать кадр в
// изолированную страницу — формулировка предиката здесь и в isolation.ts одна
// (см. `supportsCredentiallessFrames`). Браузеры без поддержки остаются
// неизолированными, и вьюер едет на транспорте `bytes` — штатный путь.

// Установка сразу боевая: держать старый воркер «ожидающим» незачем — его кэши
// всё равно подлежат удалению, а новая версия страницы к ним не обращается.
self.addEventListener('install', (event) => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      // Сносим все кэши без разбора: сам воркер не кэширует ничего, так что
      // любой найденный кэш оставлен прежней версией сайта и указывает на её
      // же сборку.
      const names = await caches.keys();
      await Promise.all(names.map((name) => caches.delete(name)));
      // claim нужен для вкладок, которые всё ещё держит прежний воркер: без него
      // он до перезагрузки продолжал бы отвечать из удаляемых кэшей.
      await self.clients.claim();
    })(),
  );
});

// ── Заголовки изоляции ───────────────────────────────────────────────────────

/**
 * Умеет ли браузер встроить кадр в изолированную страницу.
 *
 * Формулировка одна на два места — здесь и в
 * `examples/viewer/src/lib/isolation.ts` (источник правды: предикат
 * экспортируется оттуда); разъехаться им нельзя, потому что воркер изолирует
 * страницу на следующем заходе, а страницы решают, поднимать ли воркер вовсе.
 *
 * Проверяется `userAgentData`, а не атрибут `credentialless` у
 * `HTMLIFrameElement`: в воркере DOM нет, `HTMLIFrameElement` тут `undefined`
 * (проверено), а API клиент-хинтов экспонировано и в Worker'ах (проверено:
 * `'userAgentData' in self.navigator === true` в Chromium). Прокси точный —
 * браузера, знающего `credentialless`, но не `userAgentData`, сегодня нет: в
 * Chromium есть и то и другое, а Firefox и Safari не знают ни одного.
 */
function supportsCredentiallessFrames() {
  return 'userAgentData' in self.navigator;
}

/**
 * Ответ с заголовками изоляции.
 *
 * Тело не буферизуем: `new Response(response.body, …)` оставляет поток потоком,
 * так что большие ассеты едут в браузер, не собираясь целиком в памяти воркера.
 *
 * COOP + COEP нужны странице демо: только они включают cross-origin isolation,
 * без которой не бывает `SharedArrayBuffer`. COEP ставится всем ответам, а не
 * только навигациям: его спрашивает и воркер-скрипт в своём собственном ответе.
 *
 * Неуспешные ответы и ответы со `status === 0` отдаются как есть: у opaque
 * (в том числе `opaqueredirect` — так выглядит редирект навигации, чей fetch
 * идёт с внутренним `redirect: manual`) заголовки переписать нельзя, а пересборка
 * 4xx/5xx отвязала бы тело от сети ради ничего.
 */
async function withIsolationHeaders(request) {
  const response = await fetch(request);
  if (response.status === 0 || !response.ok) {
    return response;
  }
  const headers = new Headers(response.headers);
  headers.set('Cross-Origin-Resource-Policy', 'cross-origin');
  headers.set('Cross-Origin-Embedder-Policy', 'require-corp');
  if (request.mode === 'navigate') {
    headers.set('Cross-Origin-Opener-Policy', 'same-origin');
  }
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

self.addEventListener('fetch', (event) => {
  // Без поддержки — не отвечаем ничего: ни заголовков, ни лишнего хопа на
  // статику. Страница остаётся неизолированной и вьюер едет на `bytes`.
  if (!supportsCredentiallessFrames()) return;

  event.respondWith(withIsolationHeaders(event.request));
});
