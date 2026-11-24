// Регистрация сабсетов Carlito в области видимости воркера.
//
// Painter ставит canvas'у `ctx.font` с именем семейства из кадра: DOCX несёт
// канонический `Carlito`, а XLSX — алиас `Calibri` (`crates/render/src/font.rs`).
// Системного Carlito/Calibri нет ни на одной целевой платформе, поэтому без
// `FontFace` браузер подставляет своё семейство, и ширины строк предпросмотра
// расходятся с метриками раскладки и с PDF, куда встроены те же сабсеты.
// Проверять `document.fonts.check` для этого бесполезно: она возвращает
// `true` и при отсутствующем шрифте — единственная надёжная проверка — сам
// замер ширины (её и гоняет прогрев).

/**
 * Минимум wasm-модуля, нужный регистрации: байты сабсета по номеру лица.
 *
 * Тип структурный, а не `typeof import('@meerrec/wasm')`: регистрации
 * нужен один экспорт, и она не должна зависеть от свежести сгенерированных
 * `pkg/*.d.ts`.
 */
export interface FontBytesSource {
  /** 0 — regular, 1 — bold, 2 — italic, 3 — bold-italic (`crates/wasm/src/font_api.rs`). */
  font_bytes(face: number): Uint8Array;
}

/**
 * Имена, под которыми canvas может увидеть сабсет. Кадры DOCX несут канонический
 * `Carlito`, а алиас `Calibri` остаётся в кадрах XLSX (`render::font`): если
 * зарегистрировать только каноническое имя, XLSX уходит в системный fallback.
 */
const CANVAS_FAMILIES = ['Carlito', 'Calibri'] as const;

/**
 * Кегли, которые прогреваются в canvas-контексте. OOXML хранит размер в
 * полупунктах, а painter передаёт в `ctx.font` f32-значение в px; Chromium
 * запоминает fallback по точной строке, поэтому одного 16px-замера недостаточно.
 * Диапазон покрывает стандартные Office-кегли от 6 до 40 pt с шагом 0.5;
 * экзотика вне диапазона — известное ограничение (см. ROADMAP, Stage 1).
 */
export const FONT_READINESS_POINTS: readonly number[] = Object.freeze(
  Array.from({ length: 69 }, (_, index) => 6 + index / 2),
);

/** `crates/xlsx::layout::PX_PER_POINT` и умножение выполняются в f32. */
const PX_PER_POINT_F32 = new Float32Array([4 / 3])[0]!;

function point_size_px_f32(point: number): number {
  return new Float32Array([new Float32Array([point])[0]! * PX_PER_POINT_F32])[0]!;
}

/**
 * Напечатать f32 так же, как `format!("{size_px}px")` в painter'е (Rust Display).
 *
 * Строка `ctx.font` кэшируется canvas'ом посимвольно: "14.666667px" и
 * "14.666666984558105px" — разные ключи, хотя числа равны как f32. Rust печатает
 * кратчайшее десятичное, которое round-trip'ится в тот же f32 (алгоритм Рю) —
 * перебор `toFixed` с проверкой через `Float32Array` воспроизводит его ровно.
 */
function format_paint_size_f32(px: number): string {
  if (Number.isInteger(px)) return String(px);
  for (const precision of [1, 2, 3, 4, 5, 6, 7, 8, 9]) {
    const candidate = px.toFixed(precision);
    if (new Float32Array([Number(candidate)])[0] === px) return candidate;
  }
  return String(px);
}

/**
 * Семейство-канарейка: уникальное имя под байтами regular, по которому
 * `waitPaintFontsVisible` отличает «FontFace разобран» от «FontFace реально
 * виден canvas-матчеру». Имени нет ни в одном кадре, в прогрев не входит.
 */
export const PAINT_FONT_PROBE_FAMILY = '__meerrec_paint_font_probe__';

/** Лицо-заглушка для замера: семейство, которого заведомо нет ни в ОС, ни у нас. */
const ABSENT_FAMILY = '__meerrec_absent_family__';

/** Тексты замера: две строки, чтобы случайное совпадение ширины фолбэка с сабсетом
 * одной строкой не прокатило. */
const PROBE_TEXTS = ['\u0424\u0439\u0411', 'Строка 0123'] as const;

/** Лица сабсета: номер в wasm и дескрипторы `FontFace`, под которыми регистрируем. */
const FACES: readonly {
  readonly face: number;
  readonly style: 'normal' | 'italic';
  readonly weight: '400' | '700';
}[] = [
  { face: 0, style: 'normal', weight: '400' },
  { face: 1, style: 'normal', weight: '700' },
  { face: 2, style: 'italic', weight: '400' },
  { face: 3, style: 'italic', weight: '700' },
];

export interface PaintFontContext {
  font: string;
  measureText(text: string): unknown;
}

/**
 * Явно разрешить все комбинации canvas-имён и начертаний именно в том
 * 2D-контексте, которым будет рисовать painter. Строки повторяют формат
 * `PaintState::font_css` (`crates/render/src/painter/state.rs`) посимвольно:
 * `{italic }{bold }{size}px {family}, sans-serif` с Rust-Display для кегля.
 * Хоть одно расхождение — и промах кэша Chromium по конкретной строке не
 * покрыт: матчер запоминает подстановку для строки, которую painter выставит
 * первой, и после подхвата `FontFace` её уже не пересматривает.
 *
 * Звать после создания контекста и каждого `resize`: установка размеров
 * сбрасывает состояние холста.
 */
export function resolvePaintFonts(ctx: PaintFontContext): void {
  for (const family of CANVAS_FAMILIES) {
    for (const bold of [false, true]) {
      for (const italic of [false, true]) {
        for (const point of FONT_READINESS_POINTS) {
          const style = italic ? 'italic ' : '';
          const weight = bold ? 'bold ' : '';
          ctx.font = `${style}${weight}${format_paint_size_f32(point_size_px_f32(point))}px ${family}, sans-serif`;
          ctx.measureText('Font readiness');
        }
      }
    }
  }
}

/**
 * Зарегистрировать сабсеты Carlito в `self.fonts` воркера либо в
 * `document.fonts`, если функция зовётся на main-потоке. У `Window` свойства
 * `fonts` нет — набор шрифтов документа живёт в `document.fonts`.
 *
 * Возвращает `true`, только если все комбинации двух canvas-имён и четырёх
 * начертаний добавлены и разобраны: тогда `ctx.font` с `Carlito` или алиасом
 * `Calibri` разрешается в один и тот же сабсет, а предпросмотр не уходит
 * в системный fallback со своими метриками. Девятое лицо — канарейка
 * `PAINT_FONT_PROBE_FAMILY` под байтами regular: она нужна
 * `waitPaintFontsVisible` и в кадрах не участвует.
 *
 * Не падает там, где API нет (недоверенные рантаймы, области видимости без
 * `FontFace`/`self.fonts`): возвращается `false` — «не зарегистрировано», и
 * предпросмотр остаётся на подстановке браузера, как до этой функции.
 *
 * @param mod — wasm-модуль с экспортом байтов (`font_bytes`, см.
 *   `crates/wasm/src/font_api.rs`); байты копируются в JS-память
 * @returns `false` — лица не зарегистрированы целиком (нет API, не отдались
 *   байты или разбор не прошёл): предпросмотр рисует подстановкой браузера
 */
export async function registerPaintFonts(mod: FontBytesSource): Promise<boolean> {
  const scope = globalThis as {
    fonts?: FontFaceSet;
    FontFace?: typeof FontFace;
  };
  const FaceCtor = scope.FontFace;
  // В воркере — `self.fonts`, на main — `document.fonts`: у `Window` свойства
  // `fonts` нет, и без второй ветки main-режим всегда рисовал бы подстановкой.
  const fonts =
    scope.fonts ?? (typeof document === 'undefined' ? undefined : document.fonts);
  if (!FaceCtor || !fonts) {
    console.warn(
      '[fonts] FontFace/fonts недоступны: предпросмотр рисует текст подстановкой браузера',
    );
    return false;
  }

  try {
    // Сначала собираем все лица, и только потом добавляем: сломанный модуль
    // или незнакомый номер лица не должны оставить набор наполовину.
    const faces = FACES.flatMap((spec) => {
      // Приведение к `BufferSource`: типы lib различают `ArrayBuffer` и
      // `SharedArrayBuffer` (`Uint8Array<ArrayBufferLike>`), а `FontFace`
      // ждёт не shared-вариант. wasm-bindgen отдаёт обычный буфер.
      const bytes = mod.font_bytes(spec.face) as BufferSource;
      return CANVAS_FAMILIES.map(
        (family) => new FaceCtor(family, bytes, { style: spec.style, weight: spec.weight }),
      );
    });
    // Канарейка — то же лицо 0 под уникальным именем: `waitPaintFontsVisible`
    // сравнивает её ширину с заведомо отсутствующим семейством, чтобы отличить
    // «load() резолвнулся» от «шрифтовый стек реально видит лицо».
    faces.push(new FaceCtor(PAINT_FONT_PROBE_FAMILY, mod.font_bytes(0) as BufferSource));
    for (const face of faces) fonts.add(face);
    // Первый кадр может собраться сразу после `init`: без этого ожидания
    // строка первого кадра ушла бы в подстановку ещё раз. Ориентир на
    // `status`/`loaded` тот же — `load()` резолвится разобранным лицом.
    await Promise.all(faces.map((face) => face.load()));
    return true;
  } catch (error) {
    console.warn('[fonts] сабсеты Carlito не зарегистрированы:', String(error));
    return false;
  }
}

/**
 * Дождаться, пока canvas-матчер реально увидит зарегистрированные лица.
 *
 * В headless-Chromium `FontFace.load()` резолвится разобранным лицом раньше,
 * чем шрифтовый стек начинает отдавать его новым строкам `ctx.font`: первая
 * отрисовка в этом окне идёт системной подстановкой fontconfig, и промах
 * закрепляется в кэше строк canvas'а на всю сессию контекста. Различить окно
 * можно только замером: у канарейки и заведомо отсутствующего семейства
 * ширины разные ровно тогда, когда матчер видит канарейку (самокалибровка —
 * метрики сабсета в код не зашиты и при пересборке подрезки не протухают).
 *
 * @param ctx — тот же 2D-контекст, которым будет рисовать painter
 * @param timeoutMs — предел ожидания; по умолчанию 10 с — с запасом против
 *   вечного фолбэка на загруженных CI-раннерах
 * @returns `true` — канарейка видна, первый кадр пойдёт в сабсеты;
 *   `false` — таймаут, вызывающий вправе упасть, а не молчать о подстановке
 */
export async function waitPaintFontsVisible(
  ctx: PaintFontContext,
  timeoutMs = 10_000,
): Promise<boolean> {
  const startedAt = performance.now();
  // Каждый замер — новая строка (свежий кегль): кэш матчера не пересматривает
  // уже встреченную строку, и повторный замер той же строки был бы бессмыслен.
  // Кегль уникален в пределах замера — в том числе между двумя probe-текстами
  // одной итерации: иначе вторая пара прочитала бы кэш первой.
  let probePoint = 32;
  const nextSizePx = (): string => {
    const sizePx = format_paint_size_f32(point_size_px_f32(probePoint));
    probePoint += 0.5;
    return sizePx;
  };
  for (;;) {
    let probeDiffers = false;
    for (const text of PROBE_TEXTS) {
      const sizePx = nextSizePx();
      ctx.font = `${sizePx}px ${PAINT_FONT_PROBE_FAMILY}, sans-serif`;
      const probeWidth = (ctx.measureText(text) as TextMetrics).width;
      ctx.font = `${sizePx}px ${ABSENT_FAMILY}, sans-serif`;
      const absentWidth = (ctx.measureText(text) as TextMetrics).width;
      if (Math.abs(probeWidth - absentWidth) > 0.01) {
        probeDiffers = true;
        break;
      }
    }
    if (probeDiffers) return true;
    if (performance.now() - startedAt >= timeoutMs) return false;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
}
