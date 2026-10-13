# Учебные материалы по Rust

Подборка из двух дорожек: **A** — онбординг команды с нуля, **B** — то, что нужно
именно этому проекту (WASM, `web-sys`, производительность, lock-free). Ниже —
что даёт каждый материал и зачем он нам. Ссылки проверены 13.10.2026.

> **Важно: GitHub-организация `rustwasm` заархивирована в июле 2025**
> ([Inside Rust, 21.07.2025](https://blog.rust-lang.org/inside-rust/2025/07/21/sunsetting-the-rustwasm-github-org/)).
> Часть ссылок в интернете ведёт на старый дом. Сами крейты `wasm-bindgen`,
> `web-sys`, `wasm-pack` и `gloo` не заброшены — сменились дом и мейнтейнеры,
> актуальный гайд живёт на https://wasm-bindgen.github.io/wasm-bindgen/.

## Дорожка A. С нуля, для онбординга команды

| Материал | Ссылка | Что даёт |
|---|---|---|
| The Book | https://doc.rust-lang.org/book/ | Официальная база: 20 глав и финальный проект. Точка входа по умолчанию. |
| The Book, русский перевод | https://doc.rust-lang.ru/book/ | Структурно полный перевод, но по изданию ~1.67 (2023): годится для входа, не для новых фич. |
| Rustlings | https://github.com/rust-lang/rustlings | Упражнения в терминале, идут параллельно книге; классика. |
| 100 Exercises to Learn Rust | https://rust-exercises.com/100-exercises/ | Luca Palmieri, обновление 2026. От владения до async REST API, есть fast track для опытных. |
| Comprehensive Rust (Google) | https://google.github.io/comprehensive-rust/ | Курс на ~4 дня с упражнениями. Лучший вариант для группового онбординга. |
| Microsoft RustTraining | https://microsoft.github.io/RustTraining/ | Семь книг (март 2026, MIT/CC-BY): «мосты» для C/C++/C#/Python и глубокие погружения (Async, Patterns, Type-Driven Correctness, Engineering Practices). Отрендеренные версии с поиском, исходники — [microsoft/RustTraining](https://github.com/microsoft/RustTraining). |
| Rust by Example | https://doc.rust-lang.org/rust-by-example/ | Аннотированные примеры; справочник, а не учебник. |
| Rustfinity | https://www.rustfinity.com/ | Упражнения в браузере без локальной установки. |
| Exercism, Rust track | https://exercism.org/tracks/rust | 90+ задач с бесплатным ревью кода от менторов. |
| Rust for JS peeps | https://github.com/yoshuawuyts/rust-for-js-peeps | Маппинг JS-идиом в Rust. |

**Пробел:** «мостовой» книги именно для JS/TS-разработчиков нет ни у Microsoft,
ни у Google — у них есть только C/C++, C# и Python. Для команды, приходящей из TS,
ближайшая замена — Rust for JS peeps плюс вводные главы The Book про владение
и заимствование.

## Дорожка B. WASM и производительность — под проект

| Материал | Ссылка | Зачем нам |
|---|---|---|
| wasm-bindgen Guide | https://wasm-bindgen.github.io/wasm-bindgen/ | Актуальный дом после переезда: типы, классы, замыкания, генерация TS-биндингов, `wasm-bindgen-test`. |
| wasm-pack Book | https://wasm-bindgen.github.io/wasm-pack/book/ | `--target bundler`/`web`, упаковка пакета — это наш `scripts/build-wasm.sh`. |
| Rust and WebAssembly Book | https://rustwasm.github.io/docs/book/ | **Архив с 07.2025.** Главы «Time Profiling» и «Shrinking .wasm Size» плюс Game of Life всё ещё лучшее структурированное введение. |
| The Rust Performance Book | https://nnethercote.github.io/perf-book/ | Профилирование, аллокации, binary size, Cargo-профили и LTO — прямо про наш бюджет размера и `criterion`-бенчи. |
| Rust Atomics and Locks | https://mara.nl/atomics/ | Mara Bos, бесплатно онлайн: memory ordering и lock-free — под наш SAB ring с `seq_writer`/`seq_reader`. |
| web-sys API | https://docs.rs/web-sys/latest/web_sys/ | `OffscreenCanvas`, `transferControlToOffscreen`; справочник по типам, которые дёргает воркер. |
| RustTraining: Rust Patterns | https://microsoft.github.io/RustTraining/rust-patterns-book/ | Pin, аллокаторы, lock-free структуры, контролируемый `unsafe`. |
| RustTraining: Engineering Practices | https://microsoft.github.io/RustTraining/engineering-book/ | CI/CD, кросс-компиляция, Miri; перекликается с нашими `cargo deny`, pedantic-clippy и MSRV 1.82. |
| CodeCrafters | https://codecrafters.io/ | Проекты с нуля (Redis, Git, shell); платно. |

Отдельно и как опция, а не рекомендация: «Wasm in Production» (Carter, 2026) —
bundle size и профилирование wasm.
