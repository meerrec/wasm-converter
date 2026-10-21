//! Генератор `packages/core/src/generated/wire.ts`.
//!
//! Запуск: `cargo run -p doc-converter-core --features ts --example export_wire`
//! (в репозитории — `pnpm gen:wire`). Порядок типов фиксирован: перестановка
//! не должна выглядеть в diff'е как изменение протокола. Генерация
//! детерминирована — CI перегенерирует файл и падает на любом расхождении.
//!
//! `TS::decl()` отдаёт объявление одного типа и ссылается на зависимости по
//! имени: их объявления печатаются здесь же, в фиксированном порядке. Если
//! зависимость потянет `import` (например, поле с `chrono`), файл не соберётся:
//! такой тип не должен пересекать границу воркера.

use std::fs;
use std::path::PathBuf;

use doc_converter_core::wire::{
    BuildStatsWire, HyperlinkInfoWire, ImageInfoWire, MarginsWire, OrientationWire, PageInfoWire,
    PageSizeWire, PaintStatsWire, PdfOptionsWire, RenderOptionsWire, SheetInfoWire, ViewportWire,
};
use ts_rs::TS;

/// Заголовок ровно один: файл целиком машинный, ручная правка поймается CI.
const HEADER: &str = "\
// Сгенерировано из Rust (`crates/core/src/wire.rs`) через ts-rs. Не править руками.
// Пересобрать: pnpm gen:wire

";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let decls = [
        PageSizeWire::decl(),
        OrientationWire::decl(),
        MarginsWire::decl(),
        PdfOptionsWire::decl(),
        ViewportWire::decl(),
        RenderOptionsWire::decl(),
        SheetInfoWire::decl(),
        PageInfoWire::decl(),
        ImageInfoWire::decl(),
        HyperlinkInfoWire::decl(),
        PaintStatsWire::decl(),
        BuildStatsWire::decl(),
    ];

    let mut out = String::from(HEADER);
    for decl in decls {
        if decl.contains("import") {
            return Err(
                format!("declaration pulls an import into the generated file: {decl}").into(),
            );
        }
        // `decl()` отдаёт объявление без `export`: дописываем его сами, чтобы
        // типы можно было импортировать из `protocol.ts` поимённо.
        out.push_str("export ");
        out.push_str(decl.trim_end());
        out.push_str("\n\n");
    }

    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/core/src/generated/wire.ts");
    fs::write(&path, out)?;
    println!("wrote {}", path.display());
    Ok(())
}
