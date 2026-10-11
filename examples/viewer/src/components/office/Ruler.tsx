import type { ReactElement } from 'react';

/**
 * Горизонтальная линейка «Слова»: сантиметровые деления и серые поля страницы.
 *
 * Деления расставлены по сетке одного сантиметра, а не растянуты на ширину
 * контейнера: при зуме линейка обязана совпадать с масштабом листа, иначе она
 * врёт. Что не влезло — обрезает `overflow: hidden` у контейнера.
 */

/** Один сантиметр при 96 dpi — та же единица, в которой считает раскладка. */
const CM_PX = 37.8;

/** Сколько сантиметров рисуем: дальше 40 см не бывает ни один лист. */
const CM_COUNT = 40;

/** Ширина полей страницы — как у документа с обычными полями. */
const MARGIN_CM = 2.5;

/**
 * @param props.zoom — масштаб вьюера: 1 — 100 %, 2 — 200 %.
 */
export function Ruler(props: { zoom?: number }): ReactElement {
  const cm = CM_PX * (props.zoom ?? 1);
  const marks: ReactElement[] = [];
  const labels: ReactElement[] = [];
  for (let half = 0; half <= CM_COUNT * 2; half += 1) {
    const whole = half % 2 === 0;
    marks.push(
      <span
        key={half}
        className={`ofc-ruler__tick${whole ? ' ofc-ruler__tick--cm' : ''}`}
        style={{ left: (half / 2) * cm }}
      />,
    );
    if (whole && half > 0) {
      labels.push(
        <span key={half} className="ofc-ruler__num" style={{ left: (half / 2) * cm }}>
          {half / 2}
        </span>,
      );
    }
  }
  return (
    <div className="ofc-ruler" aria-hidden="true">
      <span className="ofc-ruler__margin" style={{ left: 0, width: MARGIN_CM * cm }} />
      <span className="ofc-ruler__margin" style={{ right: 0, width: MARGIN_CM * cm }} />
      {marks}
      {labels}
    </div>
  );
}
