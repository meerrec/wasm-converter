# @meerrec/eurooffice

Типизированный клиент Embed API редактора Euro-Office (форк ONLYOFFICE) поверх iframe
`/editor.html?embed=1`: команды уходят в iframe через `postMessage`, события приходят
обратно. Оборачивает то, что иначе пришлось бы повторять на каждом вызове: накопить
команды до готовности редактора, пронумеровать их, сматчить ответы по id, отсеять
запоздавшие ответы отменённых команд и передать крупный файл без копирования.

Контракт — `docs/embed-api.md` репозитория [ranuts/document](https://github.com/ranuts/document);
сверка с бандлом `assets/editor-*.js`: команды редактор принимает только от
непосредственного родителя, ответные события повторяют id команды (`T("document:opened", …, t.id)`),
`document:ready` — широковещательное (без id).

Сам редактор в пакет не входит: iframe и статику сайта (`editor.html`, `assets/`,
`sdkjs/`, `fonts/`) раздаёт потребитель. Пакет знает только форму сообщений и адрес,
по которому её слушать.

## Вызов

```ts
import { createEditorClient, describeEditorError } from '@meerrec/eurooffice';
import type { EditorEvent } from '@meerrec/eurooffice';

// iframe с src="/editor.html?embed=1" уже в разметке страницы
const frame = document.querySelector<HTMLIFrameElement>('#editor')!;
const client = createEditorClient(frame); // origin по умолчанию — свой собственный

client.onEvent((event: EditorEvent) => {
  switch (event.type) {
    case 'document:ready':
      // редактор поднялся; открывать можно и раньше — команда подождёт в очереди
      break;
    case 'document:opened':
      console.log('открыт', event.payload.fileName);
      break;
    case 'document:error': {
      // payload сужен по type: code и message — как их объявил пакет
      const { code, message } = event.payload;
      console.warn(message, code === undefined ? '' : `(${code}: ${describeEditorError(code)})`);
      break;
    }
  }
});

// файл читает родитель и отдаёт байтами — ни CORS, ни сети для этого не нужно
const bytes = await file.arrayBuffer();
client.openBuffer(file.name, bytes, true); // readonly

// спустя время — выгрузка правок; ответ придёт событием document:saved
client.save({ targetExt: 'docx' });
```

Ответ на команду приходит тем же событием, что и широковещательные: `document:state`
на `getState()`, `document:saved` (с `file`) на `save()`, `document:error` на неудачу.

## Гарантии

- **Очередь до готовности.** Команды, отправленные до `document:ready`, ждут его в
  очереди и уходят после — вызывающему не нужно ждать ready самому.
- **Свой id на команду.** Идентификатор (`eo-<время>-<номер>`) назначается клиентом
  и повторяется в ответном событии, поэтому ответы различаются.
- **Отмена устаревших.** Новое открытие/сохранение/запрос состояния снимает с учёта
  неотвеченные команды того же рода: запоздавший ответ отменённой команды
  (`supersede`) слушателю не доставляется. Событие без id или с чужим id доставляется
  как есть — редактор может не вернуть id, и ответ потерять дороже, чем показать.
- **Отсев по источнику.** Принимаются только сообщения от `frame.contentWindow` и
  origin'а редактора (свой origin или `options.origin`).
- **Transferables.** Для `document:open-buffer` подложка байтов передаётся
  transfer'ом, если `Uint8Array` покрывает её целиком, — крупный файл не копируется.
  Срез (`subarray`) покрывает не целиком, поэтому копируется: transfer отдал бы
  редактору чужие байты.
- **Неизвестное не теряется молча.** Будущие события `document:*` не интерпретируются,
  но логируются в консоль (`kind: 'unknown'`). Посторонние сообщения (не `document:*`)
  игнорируются.

## Чего пакет не делает

- **Не поднимает редактор.** iframe и статику сайта раздаёт потребитель; пакет не
  создаёт iframe и не знает его URL — только `contentWindow` переданного элемента.
- **Не знает про CORS.** `openBuffer` принимает уже прочитанные байты: файл читает
  родитель. `openUrl` — исключение, там файл тянет браузер, и CORS на ответ обязан
  пропустить сам сервер.
- **Не умеет отписываться от `window`.** Слушателя `message` клиент не снимает —
  он живёт столько же, сколько клиент, а клиент — сколько iframe; функции
  `dispose` в API нет. `onEvent` возвращает отписку только от событий этого
  клиента.

## Расшифровка ошибок

`describeEditorError(code)` отдаёт человекочитаемый текст для известных кодов:
`-82` — не удалось сконвертировать (файл повреждён, зашифрован или вариант формата не
поддержан), `-85` — содержимое не совпадает с расширением файла. Неизвестный код
(в том числе отсутствующий) — общая фраза с кодом, чтобы ошибка не была молчаливой.

## Разработка

```bash
pnpm -F @meerrec/eurooffice test      # vitest в jsdom: iframe подменён моком
pnpm -F @meerrec/eurooffice typecheck
pnpm -F @meerrec/eurooffice build     # tsup: dist/ с декларациями
```

Тесты не поднимают редактор: `postMessage` iframe'а — мок, входящие события —
`MessageEvent` на `window` с подставленными `origin` и `source`.
