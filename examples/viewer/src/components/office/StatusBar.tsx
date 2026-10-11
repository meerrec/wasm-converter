import type { ReactElement, ReactNode } from 'react';

/**
 * Строка состояния приложения: слева — то, что относится к документу
 * (страница, подсказка), в центре — метрики кадров, справа — управление
 * видом и сообщения.
 *
 * `children` уезжает в середину: там стоят счётчики вьюера, которые меняются
 * десятки раз в секунду и пишутся в DOM мимо React.
 */
export function StatusBar(props: { left?: ReactNode; right?: ReactNode; children?: ReactNode }): ReactElement {
  return (
    <footer className="ofc-statusbar">
      <span className="ofc-side ofc-side--left">{props.left}</span>
      <span className="ofc-statusbar__meters">{props.children}</span>
      <span className="ofc-side ofc-side--right">{props.right}</span>
    </footer>
  );
}
