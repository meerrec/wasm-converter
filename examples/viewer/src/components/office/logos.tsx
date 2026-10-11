import type { ReactElement } from 'react';

/**
 * Логотипы приложений каркаса: «Слово» (документ) и «ДваИксель» (книга).
 *
 * Нарисованы своей геометрией, а не сняты с товарных знаков Microsoft: у обоих
 * общий приём — скруглённый квадрат и белый знак внутри, — но знаки разные
 * (литера «С» против «XXL»), и пропорции заданы здесь, а не скопированы.
 */

/** Размер холста логотипа: viewBox один на оба знака, чтобы их было видно рядом. */
const VIEW = 48;

/** Радиус скругления квадрата: доля стороны, общая у обоих логотипов. */
const ROUND = 10;

/**
 * Логотип «Слово»: синий документ с белой литерой «С».
 *
 * @param props.size — сторона квадрата в пикселях (по умолчанию 24).
 */
export function LogoWord(props: { size?: number }): ReactElement {
  const size = props.size ?? 24;
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${VIEW} ${VIEW}`}
      role="img"
      aria-label="Слово"
    >
      <rect width={VIEW} height={VIEW} rx={ROUND} fill="#185abd" />
      {/* «С» — разомкнутое кольцо: длинная дуга против часовой стрелки,
          концы на одном x, как у буквы. */}
      <path
        d="M31.5 17.2 A 9.5 9.5 0 1 0 31.5 30.8"
        fill="none"
        stroke="#ffffff"
        strokeWidth={5}
        strokeLinecap="round"
      />
    </svg>
  );
}

/**
 * Логотип «ДваИксель»: зелёная книга с белыми литерами «XXL».
 *
 * @param props.size — сторона квадрата в пикселях (по умолчанию 24).
 */
export function LogoExcel(props: { size?: number }): ReactElement {
  const size = props.size ?? 24;
  // Буквы нарисованы штрихами, а не текстом: так знак не зависит от того,
  // какие шрифты есть в системе, и одинаков на всех платформах.
  const stroke = {
    fill: 'none',
    stroke: '#ffffff',
    strokeWidth: 3.4,
    strokeLinecap: 'round',
    strokeLinejoin: 'round',
  } as const;
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${VIEW} ${VIEW}`}
      role="img"
      aria-label="ДваИксель"
    >
      <rect width={VIEW} height={VIEW} rx={ROUND} fill="#107c41" />
      <g {...stroke}>
        <path d="M11 17 L18.5 31" />
        <path d="M18.5 17 L11 31" />
        <path d="M21.5 17 L29 31" />
        <path d="M29 17 L21.5 31" />
        <path d="M32 17 L32 31 L39 31" />
      </g>
    </svg>
  );
}
