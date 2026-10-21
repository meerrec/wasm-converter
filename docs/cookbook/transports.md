# Рецепты: транспорты кадра

Кадр собирается в Web Worker'е и рисуется либо там же (`OffscreenCanvas`),
либо на main тем же Rust-painter'ом, когда браузер не умеет
`transferControlToOffscreen` — это main-режим
([ADR-0029](../adr/0029-main-thread-render-backend.md)). У движка два
равноправных транспорта: `sab` — ring из двух слотов в `SharedArrayBuffer`,
`bytes` — два `ArrayBuffer`, которые воркер заполняет по очереди. API, кадры
и экспорт одинаковы на обоих путях; отличается транспорт и, вслед за ним,
скорость.

---

### <a id="tr-01"></a>Рецепт 1. Выбор транспорта: `auto` и `viewer.transport`

**Задача.** Узнать, каким путём поехали кадры, ничего не задавая.

```ts
import { createXlsxViewer } from '@meerrec/core';

const viewer = await createXlsxViewer(host, {
  workerUrl,
  // transport: 'auto' — значение по умолчанию, его можно не писать.
});
await viewer.open(bytes);

console.log(viewer.transport);            // 'sab' | 'bytes'
console.log(crossOriginIsolated);         // true ⇔ страница отдана с COOP/COEP
```

**Под капотом.** При `auto` воркер сам решает по `crossOriginIsolated`:
есть COOP/COEP — `sab`, нет — `bytes`; интерфейсу знать путь не нужно,
поле `transport` существует для диагностики и тестов.

---

### <a id="tr-02"></a>Рецепт 2. Явный `sab` без заголовков — ошибка инициализации

**Задача.** Заставить страницу использовать SAB и получить внятную ошибку,
если заголовков нет.

```ts
try {
  const viewer = await createXlsxViewer(host, {
    workerUrl,
    transport: 'sab', // требует crossOriginIsolated
  });
  await viewer.open(bytes);
} catch (error) {
  // Без COOP/COEP промис отклоняется здесь, на инициализации, а не
  // молча подменяется транспортом bytes.
  console.error(error);
}
```

**Под капотом.** Явный `sab` без заголовков — ошибка инициализации по
контракту (`ViewerOptions.transport` в `offscreen.ts`), а не тихая подмена:
так отладка не тратит время на «почему SAB не включился».

---

### <a id="tr-03"></a>Рецепт 3. Явный `bytes`: гарантированно без SAB

**Задача.** Прогнать страницу буферным путём, даже если COOP/COEP есть.

```ts
const viewer = await createDocxViewer(host, {
  workerUrl,
  docxWasm,
  transport: 'bytes', // два ArrayBuffer + transfer вместо кольца
});
await viewer.open(bytes);
console.assert(viewer.transport === 'bytes');
```

**Под капотом.** На `bytes` воркер заполняет два буфера по очереди и
передаёт их transferables — двойная буферизация та же, что у ring'а, а
собирает кадр ровно та же Rust-функция; явный выбор полезен в тестах и для
воспроизведения багов embed-окружений.

---

### <a id="tr-04"></a>Рецепт 4. COOP/COEP: как отдать заголовки

**Задача.** Включить SAB-путь на своём сервере.

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

**Под капотом.** Заголовки ставит тот, кто отдаёт **страницу** (библиотека
себе их поставить не может); без них `crossOriginIsolated` ложен, и воркер
выбирает `bytes` — см. рецепт Vite в [integration.md](integration.md#in-02).

---

### <a id="tr-05"></a>Рецепт 5. Embed без SAB: iframe, расширение, GitHub Pages

**Задача.** Встроить просмотрщик туда, где заголовки не под контролем.

```ts
// iframe на чужом сайте, браузерное расширение, Electron/webview, GitHub
// Pages: заголовки контролирует хост, а не вы. Ничего менять не нужно —
// тот же код, тот же API; без crossOriginIsolated воркер сам выберет bytes.
const viewer = await createXlsxViewer(host, { workerUrl });
await viewer.open(bytes);
console.log(viewer.transport); // 'bytes'
```

**Под капотом.** Путь без SAB — полноценный, а не fallback: так требует
[ADR-0021](../adr/0021-frame-transports.md); на GitHub Pages заголовки ставит
корневой service worker сайта, который включается на первом заходе, отчего
страница один раз перезагружается (`docs/workers.md`).

---

### <a id="tr-06"></a>Рецепт 6. Что даёт SAB: ring и отбрасывание кадров

**Задача.** Объяснить (или выбрать), зачем вообще нужен SAB-путь.

```ts
import { createXlsxViewer, type FrameTransport } from '@meerrec/core';

// Простейший замер: прогнать один и тот же файл обоими путями и сравнить.
// 'sab' требует COOP/COEP — на странице без заголовков probe('sab') упадёт
// на инициализации, поэтому сравнение честно только в crossOriginIsolated.
async function probe(
  transport: 'sab' | 'bytes',
): Promise<{ transport: FrameTransport; avgPaintMs: number }> {
  const viewer = await createXlsxViewer(host, { workerUrl, transport });
  await viewer.open(bytes);
  let sum = 0;
  let count = 0;
  viewer.onTick((s) => {
    sum += s.paintMs;
    count += 1;
  });
  for (let i = 0; i < 50; i += 1) viewer.scrollTo(0, i * 64);
  await new Promise((resolve) => setTimeout(resolve, 500));
  const result = { transport: viewer.transport, avgPaintMs: sum / count };
  viewer.destroy();
  return result;
}

console.log(await probe('sab'));
console.log(await probe('bytes'));
```

**Под капотом.** Разница — в скорости, а не в поведении: тот же painter,
та же сборка; раскладка кольца продублирована и обязана совпадать
(`crates/render/src/sab/ring.rs` ↔ `packages/core/src/sab/protocol.ts`),
история решения — [ADR-0004](../adr/0004-sab-acceleration.md).

---

### <a id="tr-07"></a>Рецепт 7. Диагностика dropped-кадров

**Задача.** Заметить, что кадры пропускаются, и понять, чем это грозит.

```ts
import type { PaintStats } from '@meerrec/core';

viewer.onTick((stats: PaintStats) => {
  if (stats.dropped) {
    // Кадр, который просили нарисовать, не нарисован: на 'sab' ring был полон
    // и промежуточный кадр пропущен, на 'bytes' готового кадра не оказалось.
    // Хосту этот кадр не показывали — следующий покажут.
    console.warn(`кадр ${stats.frameId} пропущен`);
  }
});
```

**Под капотом.** `dropped` — честный флаг дросселирования: на SAB нет
свободного слота — кадр дропается, а не копится в очереди; на буферах
значение говорит, что painter'у не досталось готового кадра.

---

### <a id="tr-08"></a>Рецепт 8. Браузер без OffscreenCanvas: `renderMode`

**Задача.** Запустить просмотрщик там, где `canvas.transferControlToOffscreen`
нет (старый webview, урезанный embed), и понять, какой режим включился.

```ts
import { createXlsxViewer } from '@meerrec/core';

const viewer = await createXlsxViewer(host, {
  workerUrl,
  // 'auto' — по умолчанию: worker при transferControlToOffscreen, иначе main.
  renderMode: 'auto',
});
await viewer.open(bytes);

console.log(viewer.renderMode); // 'worker' | 'main'
console.log(viewer.transport);  // в main-режиме всегда 'bytes'
```

**Под капотом.** В main-режиме воркер не забирает canvas: он по-прежнему
парсит файл и собирает DisplayList, но шлёт его на main сообщением `frame` в
двух `ArrayBuffer`; main рисует тем же Rust-painter'ом (`init_painter_main`),
коалесцирует кадры по `requestAnimationFrame` и возвращает буферы
`frame-ack`. `tick` в этом режиме эмитит main. Явный `renderMode: 'worker'`
без возможности — ошибка инициализации, `renderMode: 'main'` вместе с
`transport: 'sab'` — ошибка конфигурации: SAB-кольцо живёт в воркере.
История решения — [ADR-0029](../adr/0029-main-thread-render-backend.md).

**Ограничение.** Main-режим — singleton: один вьюер на страницу. Второй
`createXlsxViewer`/`createDocxViewer` без OffscreenCanvas отклоняется с
ошибкой «main-режим: один вьюер на страницу». `exportPng` в этом режиме
считается локально через `HTMLCanvasElement.toBlob`, `exportPdf` и текст —
по-прежнему в воркере.

---

Ссылки: [ADR-0021](../adr/0021-frame-transports.md) — равноправие транспортов,
[ADR-0029](../adr/0029-main-thread-render-backend.md) — main-режим без
OffscreenCanvas, [ADR-0004](../adr/0004-sab-acceleration.md) — SAB-ускорение,
`docs/workers.md` — COOP/COEP и корневой service worker.
