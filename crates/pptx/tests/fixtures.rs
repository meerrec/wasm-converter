//! Сквозные тесты на коммитируемых фикстурах `test-fixtures/pptx`:
//! каждая из них обязана открыться, а у текстовых — дать непустой конспект.
//!
//! Фикстуры генерирует `scripts/generate_pptx_fixtures.ts` детерминированно
//! (повторный прогон даёт те же байты), поэтому тесты зависят от содержимого,
//! а не от таймстампов архива.

use std::path::Path;

use doc_converter_pptx::export::to_text;
use doc_converter_pptx::model::ShapeKind;

/// Все коммитируемые фикстуры: имя файла, ожидаемое число слайдов и есть ли
/// в модели текст.
const FIXTURES: &[(&str, usize, bool)] = &[
    ("simple/two_slides.pptx", 2, true),
    ("shapes/geometry.pptx", 1, true),
    ("images/picture.pptx", 1, true),
    ("charts/bar.pptx", 1, true),
    ("text/bullets.pptx", 1, true),
];

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("test-fixtures")
        .join("pptx")
        .join(name)
}

/// Каждая фикстура открывается, число слайдов и состав модели совпадают с
/// описанием генератора.
#[test]
fn every_fixture_opens() {
    for (name, slides, has_text) in FIXTURES {
        let path = fixture_path(name);
        let pres = doc_converter_pptx::open(std::fs::read(&path).unwrap_or_else(|err| {
            panic!("фикстура {name} читается: {err}");
        }))
        .unwrap_or_else(|err| panic!("фикстура {name} открывается: {err}"));
        assert_eq!(pres.slide_count(), *slides, "фикстура {name}: слайды");
        let any_text = pres.slides.iter().any(|slide| {
            slide
                .shapes
                .iter()
                .any(|shape| matches!(shape.kind, ShapeKind::Text(_)))
        });
        assert_eq!(any_text, *has_text, "фикстура {name}: текст в модели");
    }
}

/// Размер слайда всех фикстур — 16:9 (умолчание генератора): 1280×720 px.
#[test]
fn fixtures_use_the_generator_slide_size() {
    for (name, _, _) in FIXTURES {
        let pres = doc_converter_pptx::open(std::fs::read(fixture_path(name)).unwrap()).unwrap();
        assert_eq!(pres.size.width_px(), 1280.0, "фикстура {name}: ширина");
        assert_eq!(pres.size.height_px(), 720.0, "фикстура {name}: высота");
    }
}

/// Конспект текстовых фикстур непустой: слайды несут текстовые рамки.
#[test]
fn text_export_is_not_empty() {
    for (name, _, has_text) in FIXTURES {
        if !has_text {
            continue;
        }
        let pres = doc_converter_pptx::open(std::fs::read(fixture_path(name)).unwrap()).unwrap();
        let text = to_text(&pres);
        assert!(
            text.contains('\n'),
            "фикстура {name}: конспект непустой, вывод: {text:?}"
        );
    }
}
