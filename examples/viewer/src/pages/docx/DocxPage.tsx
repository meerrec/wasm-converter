import type { ReactElement } from 'react';

/**
 * Страница DOCX («Слово»). Заглушка этого слайса: маршрут и каркас уже есть,
 * вьюер, лента и подвал появляются следующим слайсом.
 */
export default function DocxPage(): ReactElement {
  return (
    <>
      <header>
        <h1>Слово</h1>
      </header>
      <main id="viewer" />
    </>
  );
}
