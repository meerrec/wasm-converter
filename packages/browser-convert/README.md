# @meerrec/browser-convert

Браузерный конвертер OOXML поверх обвязки `@meerrec/x2t` (x2t.wasm, конвертер
форматов из состава ONLYOFFICE) плюс то, что x2t сама не умеет: препроцессинг OOXML
перед конвертацией, извлечение медиа из контейнеров и сохранение результата на диск.

Сами артефакты `x2t.js`/`x2t.wasm` пакет **не поставляет** — оба URL задаёт вызывающий
(см. [`@meerrec/x2t`](../x2t/README.md); готовые файлы релиза v9.3.2+3 лежат в
[`packages/x2t/vendor/`](../x2t/vendor/README.md), Git LFS). Пакет не знает ни раскладки
каталогов потребителя, ни откуда приезжают 39 МиБ wasm.

## Конвертация в воркере

Конвертация живёт только в воркере: сборка x2t без asyncify, `main1` крутится в потоке
до конца вызова, и на главном потоке страница зависнет. Пакет отдаёт готовый вход
воркера — точку `@meerrec/browser-convert/worker`:

```ts
const worker = new Worker(new URL('@meerrec/browser-convert/worker', import.meta.url), {
  type: 'module',
});

const id = 0;
const bytes = await file.arrayBuffer();
worker.postMessage(
  {
    kind: 'convert',
    id,
    scriptUrl: '/x2t/x2t.js',
    wasmUrl: '/x2t/x2t.wasm',
    bytes, // уходит через transfer
    from: 'docx',
    to: 'odt',
    fileName: file.name,
  },
  [bytes], // тот же ArrayBuffer — в transfer-списке
);

worker.addEventListener('message', (event: MessageEvent<ConvertWorkerResponse>) => {
  if (event.data.id !== id) return;
  if (event.data.kind === 'result') {
    const { bytes, code, stderr, convertMs } = event.data;
    // bytes — ArrayBuffer выхода; code 0 — успех (код пакет НЕ фильтрует)
  } else {
    // event.data.message / event.data.code
  }
});
```

Протокол и типы (`ConvertWorkerRequest`, `ConvertWorkerResponse`) экспортируются из той
же точки `@meerrec/browser-convert/worker`; сама логика — фабрика
`createConvertWorkerHandler(post)`, слушатель реального воркера висит на ней же.

- Модуль x2t поднимается лениво на первом запросе: 38 МБ не должны быть ценой входа,
  если воркер создали на всякий случай.
- `scriptUrl`/`wasmUrl` берутся из **первого** сообщения и запоминаются; URL из
  последующих сообщений игнорируются.
- Отказ подъёма не кэшируется: следующий запрос пробует `loadX2t` снова (с URL'ами
  первого сообщения) — один сетевой сбой не ломает конвертацию навсегда.
- Байты результата уезжают через transfer, а не копией; stderr рантайма — в `stderr`
  ответа, отказ подъёма/падение рантайма — ответ `error`, stderr ошибки дописан в
  `message` отдельной строкой.

## API

| Экспорт | Что делает |
|---|---|
| `X2TConverter` | Класс-обёртка: `init()` поднимает рантайм, `convert()` гоняет файл. Для встраивания, где воркер уже есть (обвязка сама живёт в воркере) |
| `createConvertWorkerHandler` | Вход воркера из точки `/worker`: обработчик `ConvertWorkerRequest` → `ConvertWorkerResponse` |
| `preprocessXlsxLineBreaks` | Снять двойное экранирование переноса строки (`&amp;#10;` → `&#10;`) по `xl/*.xml` |
| `preprocessPptx` | Фиксы PPTX: срез `showMasterPhAnim` в notesSlides/notesMasters, инжект недостающих `docProps/app.xml` и `core.xml` с отношениями |
| `preprocessDocxRuby` | Развернуть `<w:ruby>` в базовый текст по `word/*.xml` (импортёр x2t роняет ruby целиком) |
| `unwrapRubyXml` | Чистая XML-трансформация ruby (низкий уровень `preprocessDocxRuby`) |
| `extractDocxMediaUrls` | Медиафайлы DOCX/XLSX/PPTX как object URLs: `{ "media/<имя>": blobUrl }` |
| `saveFileToDisk` | Скачать `Uint8Array` файлом (`<a download>` + object URL) |
| `documentMimeType` | MIME-тип по расширению документа |

Типы: `ConvertOptions`, `ConvertResult`, `X2TConverterOptions`, `ConvertWorkerRequest`,
`ConvertWorkerResponse`, `SaveFileOptions`, `ZipEntry`, `ZipRewriteOptions`.

Все препроцессоры без изменений возвращают исходные байты (архив не пересобирается) и
не валят вход по ошибкам: битые записи медиа пропускаются, порча архива бросает только
у zip-механики.

## Примеры

### Препроцессинг перед конвертацией

```ts
import { preprocessDocxRuby } from '@meerrec/browser-convert';

// Импортёр x2t роняет <w:ruby> вместе с базовым словом — снимаем ruby до конвертации.
const fixed = await preprocessDocxRuby(await file.arrayBuffer());
// дальше fixed — в convert воркера или X2TConverter
```

### Медиа-экстракция с ревокацией

```ts
import { extractDocxMediaUrls } from '@meerrec/browser-convert';

const urls = await extractDocxMediaUrls(bytes); // { 'media/image1.png': blobUrl }
// …показать <img src={urls['media/image1.png']} />…
// Вызывающий обязан ревокать: URL.createObjectURL держит память, пока URL жив.
for (const url of Object.values(urls)) URL.revokeObjectURL(url);
```

### X2TConverter внутри собственного воркера

```ts
import { X2TConverter } from '@meerrec/browser-convert';

const converter = new X2TConverter();
await converter.init({ scriptUrl: '/x2t/x2t.js', wasmUrl: '/x2t/x2t.wasm' });
const { bytes, code, stderr, mimeType } = await converter.convert(file, 'odt');
```

`init` повторно бросает: glue-скрипт исполняется глобально и кладёт единственный
`Module` на область видимости — второй init в той же области невозможен.

## Поддержка направлений (честная матрица)

Измерено на сборках **v9.3.2+3** (наша вендоренная) и **9.3.5**
(`@kreijstal/eurooffice`, edit.chaxus.com, прогон 14.10.2026):

| Направление | Код | Комментарий |
|---|---|---|
| Внутри семейства: `docx↔odt/rtf/txt`, `xlsx↔ods/csv` | 0 | Рабочий путь пакета |
| Кросс-семейные (`xlsx→docx`, `docx→xlsx/pptx/csv`, …) | 88 | Конвертер отказывает сам |
| Рендер-цели: `pdf`, `html`, `png`, `jpg` | 80 | Движка раскладки в wasm нет — [`docs/adr/0024-x2t-rendering-backend.md`](../../docs/adr/0024-x2t-rendering-backend.md) |

Пакет код возврата **не фильтрует**: `code` всегда доезжает до вызывающего, решает он.

Про pdf-цели: в апстриме раскладку страницы делает JS-движок sdkjs, вырезанный из
wasm-сборки заглушкой. У `@ranuts/converter` pdf работает только через canvas-поток из
их редактора (`m_nFormatFrom=8196`, уже отрисованный документ), а не прямым
`docx → pdf` — такого пути у нас нет.

Оговорки на реальных документах: битый `word/footer8.xml` в выходе конвертации,
падения `x2t crashed during conversion: memory access out of bounds` на тяжёлых
документах (подробности прогонов — [`experiments/x2t-spike/README.md`](../../experiments/x2t-spike/README.md)).

## Готчи

- **`convert` блокирует поток.** x2t без asyncify: на главном потоке страница
  зависнет — только воркер (точка `/worker` или `X2TConverter` внутри собственного).
- **`init` повторно бросает.** Модуль-синглтон на область видимости: одна загрузка
  glue на воркер, не больше.
- **Ревокать media-URLs.** `extractDocxMediaUrls` возвращает object URLs —
  обязанность вызывающего вызвать `URL.revokeObjectURL`, иначе память держится.
- **Zip читается по central directory.** Размеры и CRC берутся только оттуда, поэтому
  читаются и архивы потоковых писателей (local header с нулями, data descriptor).
- **Вход конвертации — только байты файла.** Медиа из контейнера x2t не достаёт и
  обратно не кладёт; работа с медиа — на вызывающем (см. `extractDocxMediaUrls`).

## Разработка

```bash
pnpm -F @meerrec/browser-convert test        # vitest: архив строится руками, x2t замокан
pnpm -F @meerrec/browser-convert typecheck
pnpm -F @meerrec/browser-convert build       # tsup: dist/ с декларациями, две точки входа
```

Тесты не поднимают wasm: рантайм x2t подменён заглушкой, вход воркера гоняется через
фабрику `createConvertWorkerHandler` без настоящего воркера. Живьём пакет проверяет
харнесс [`experiments/x2t-spike/`](../../experiments/x2t-spike/).
