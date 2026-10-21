import { lazy, Suspense, type ReactElement } from 'react';
import { Navigate, Route, Routes } from 'react-router-dom';
import { BootFallback } from './components/BootFallback.js';

// Страницы грузятся лениво: код одной не нужен на другой. Статический импорт
// втянул бы в общий бандл обе — разметку DOCX на XLSX-странице и наоборот, —
// а пользователь открывает формат за раз.
const DocxPage = lazy(() => import('./pages/docx/DocxPage.js'));
const XlsxPage = lazy(() => import('./pages/xlsx/XlsxPage.js'));

/**
 * Маршруты примера: DOCX — на `/docx`, XLSX — на `/xlsx`.
 */
export default function App(): ReactElement {
  // Страница подгружается чанком, поэтому каждая обёрнута в `Suspense`: без
  // него отрисовка ждала бы промис и роняла React.
  const page = (element: ReactElement): ReactElement => (
    <Suspense fallback={<BootFallback />}>{element}</Suspense>
  );
  return (
    <Routes>
      <Route path="/" element={<Navigate to="/docx" replace />} />
      <Route path="/docx" element={page(<DocxPage />)} />
      <Route path="/xlsx" element={page(<XlsxPage />)} />
      {/* Незнакомый (или старый) адрес ведёт на DOCX — примера это не роняет. */}
      <Route path="*" element={<Navigate to="/docx" replace />} />
    </Routes>
  );
}
