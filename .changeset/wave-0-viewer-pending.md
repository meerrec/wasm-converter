---
"@meerrec/core": patch
---

Исправлены висящие промисы `hitTest`/`hyperlinkAt`: при `destroy` и ошибке
воркера запросы отклоняются, а клик по гиперссылке больше не оставляет
unhandled rejection.
