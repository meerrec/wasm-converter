import type { ReactElement } from 'react';

/**
 * Страница XLSX («ДваИксель»). Заглушка этого слайса: маршрут и каркас уже
 * есть, вьюер, лента и подвал появляются следующим слайсом.
 */
export default function XlsxPage(): ReactElement {
  return (
    <>
      <header>
        <h1>ДваИксель</h1>
      </header>
      <main id="viewer" />
    </>
  );
}
