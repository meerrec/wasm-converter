import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { HashRouter } from 'react-router-dom';
import App from './App.js';
import { ensureCrossOriginIsolation } from './lib/isolation.js';
import './styles.css';

// Запускается на любой странице демо, но приложение её не ждёт: в dev и preview
// функция выходит сразу (изоляцию даёт Vite), а на Pages перезагрузка ради
// заголовков обрывает текущий запуск — откладывать её незачем.
void ensureCrossOriginIsolation();

// basename у HashRouter НЕ передаётся намеренно: он срезается из пути после
// `#`, и с `/wasm-converter/` не совпал бы ни один маршрут — на Pages пример
// открывался бы пустым. Серверный путь держит Vite (`base`), роутеру достаётся
// только то, что стоит после решётки.
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <HashRouter>
      <App />
    </HashRouter>
  </StrictMode>,
);
