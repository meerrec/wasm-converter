// Регистрация сабсетов Carlito в области видимости воркера.
//
// Painter ставит canvas'у `ctx.font` с именем семейства из кадра, а имя там
// каноническое — `Carlito` (`crates/render/src/font.rs::DEFAULT_FONT_FAMILY`).
// Системного Carlito нет ни на одной целевой платформе, поэтому без
// `FontFace` браузер подставляет своё семейство, и ширины строк предпросмотра
// расходятся с метриками раскладки и с PDF, куда встроены те же сабсеты.
// Проверять `document.fonts.check` для этого бесполезно: она возвращает
// `true` и при отсутствующем шрифте — единственная надёжная проверка — сам
// замер ширины (её и гоняет e2e).

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

/** Имя семейства: ровно то, что уезжает в `ctx.font` из кадров painter'а. */
const FAMILY = 'Carlito';

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

/**
 * Зарегистрировать сабсеты Carlito в `self.fonts` области воркера.
 *
 * Возвращает `true`, только если все четыре лица добавлены и разобраны:
 * тогда `ctx.font = "…px Carlito"` разрешается в настоящий сабсет, а ширины
 * предпросмотра сходятся с раскладкой и PDF.
 *
 * Не падает там, где API нет (недоверенные рантаймы, области видимости без
 * `FontFace`/`self.fonts`): возвращается `false` — «не зарегистрировано», и
 * предпросмотр остаётся на подстановке браузера, как до этой функции.
 *
 * Звать до первого кадра: в Chromium промах по конкретной строке `ctx.font`
 * (семейство и кегль) запоминается на сессию воркера и после регистрации уже
 * не пересматривается — текст, нарисованный до вызова, так и останется на
 * метриках подстановки.
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
  const fonts = scope.fonts;
  if (!FaceCtor || !fonts) {
    console.warn(
      '[fonts] FontFace/self.fonts недоступны: предпросмотр рисует текст подстановкой браузера',
    );
    return false;
  }

  try {
    // Сначала собираем все лица, и только потом добавляем: сломанный модуль
    // или незнакомый номер лица не должны оставить набор наполовину.
    const faces = FACES.map((spec) => {
      // Приведение к `BufferSource`: типы lib различают `ArrayBuffer` и
      // `SharedArrayBuffer` (`Uint8Array<ArrayBufferLike>`), а `FontFace`
      // ждёт не shared-вариант. wasm-bindgen отдаёт обычный буфер.
      const bytes = mod.font_bytes(spec.face) as BufferSource;
      return new FaceCtor(FAMILY, bytes, { style: spec.style, weight: spec.weight });
    });
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
