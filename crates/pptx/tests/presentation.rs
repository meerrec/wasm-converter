//! Сквозные тесты разбора PresentationML: пакет собирается в памяти, как это
//! делает PowerPoint, и проходит весь путь — разбор → раскладка → рендер →
//! текстовый экспорт.

use std::io::{Cursor, Write};

use doc_converter_core::zip_limits::ZipLimits;
use doc_converter_core::SegmentedBuffer;
use doc_converter_pptx::export::to_markdown;
use doc_converter_pptx::layout::layout_slide;
use doc_converter_pptx::model::ShapeKind;
use doc_converter_pptx::plugin::PptxPlugin;
use doc_converter_pptx::render;
use doc_converter_render::display_list::{DisplayList, DrawCommand};
use doc_converter_render::plugin::FormatPlugin;
use doc_converter_render::viewport::Viewport;

/// 1×1 красный PNG — картинка фикстуры.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x62, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

/// Минимальный пакет PPTX с титульным плейсхолдером, картинкой и диаграммой.
fn package() -> Vec<u8> {
    let parts: &[(&str, &[u8])] = &[
        ("[Content_Types].xml", br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/></Types>"#),
        ("_rels/.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#),
        ("ppt/presentation.xml", br#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/></p:presentation>"#),
        ("ppt/_rels/presentation.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#),
        ("ppt/slideMasters/slideMaster1.xml", br#"<p:sldMaster xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:bg><p:bgPr><a:solidFill xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:srgbClr val="F2F2F2"/></a:solidFill></p:bgPr></p:bg><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:off x="838200" y="365125"/><a:ext cx="10515600" cy="1325563"/></a:xfrm></p:spPr></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Body"/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:off x="838200" y="1825625"/><a:ext cx="10515600" cy="4351338"/></a:xfrm></p:spPr></p:sp></p:spTree></p:cSld><p:txStyles><p:titleStyle><a:lvl1pPr xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:defRPr sz="4400"/></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:defRPr sz="1800"/></a:lvl1pPr></p:bodyStyle></p:txStyles></p:sldMaster>"#),
        ("ppt/slideMasters/_rels/slideMaster1.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>"#),
        ("ppt/theme/theme1.xml", br#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:accent1><a:srgbClr val="4472C4"/></a:accent1></a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri Light"/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#),
        ("ppt/slideLayouts/slideLayout1.xml", br#"<p:sldLayout xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree/></p:cSld></p:sldLayout>"#),
        ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#),
        ("ppt/slides/slide1.xml", br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld name="Deck 1"><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="4" name="Title 1"/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"/><a:p xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:r><a:t>Hello Slides</a:t></a:r></a:p></p:txBody></p:sp><p:pic><p:nvPicPr><p:cNvPr id="5" name="Picture"/></p:nvPicPr><p:blipFill><a:blip xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" r:embed="rId2"/></p:blipFill><p:spPr><a:xfrm xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:off x="1000000" y="3000000"/><a:ext cx="2000000" cy="1500000"/></a:xfrm></p:spPr></p:pic><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="6" name="Chart"/></p:nvGraphicFramePr><p:xfrm><a:off xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" x="4000000" y="1000000"/><a:ext xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" cx="6000000" cy="4000000"/></p:xfrm><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId3"/></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#),
        ("ppt/slides/_rels/slide1.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>"#),
        ("ppt/charts/chart1.xml", br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:title><c:tx><c:rich><a:p xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:r><a:t>Sales</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:ser><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>2024</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>20</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#),
        ("ppt/media/image1.png", PNG),
    ];
    zip(parts)
}

/// ZIP в памяти: части идут в том же порядке, что в списке.
fn zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in parts {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }
    buf
}

#[test]
fn parses_package_end_to_end() {
    let pres = doc_converter_pptx::open(package()).unwrap();

    assert_eq!(pres.slide_count(), 1);
    assert_eq!(pres.slides[0].name, "Deck 1");
    assert_eq!(pres.size.width_px(), 1280.0);
    assert_eq!(pres.size.height_px(), 720.0);
    assert_eq!(pres.theme.major_font, "Calibri Light");
    assert_eq!(pres.theme.minor_font, "Calibri");
    assert_eq!(pres.theme.colors.accent[0].to_u32(), 0x4472C4);
    assert_eq!(pres.images.len(), 1);
    assert_eq!(pres.images[0].mime, "image/png");
    assert!(pres.warnings.is_empty(), "{:?}", pres.warnings);
    // Титульный плейсхолдер без своей геометрии наследует рамку мастера.
    let title = pres.slides[0]
        .shapes
        .iter()
        .find(|shape| shape.name == "Title 1")
        .unwrap();
    assert_eq!(title.frame.x, 838_200);
    assert_eq!(title.frame.w, 10_515_600);
    let ShapeKind::Text(text_box) = &title.kind else {
        panic!("титул должен быть текстовой рамкой");
    };
    assert_eq!(text_box.body.paragraphs[0].runs[0].text, "Hello Slides");
    // Каскад: стиль уровня titleStyle мастера → 44 pt.
    assert_eq!(text_box.body.paragraphs[0].runs[0].props.size, Some(44.0));

    // Картинка и диаграмма.
    let picture = pres.slides[0]
        .shapes
        .iter()
        .find(|shape| shape.name == "Picture")
        .unwrap();
    assert!(matches!(
        picture.kind,
        ShapeKind::Picture { image_id: Some(0) }
    ));
    let chart = pres.slides[0]
        .shapes
        .iter()
        .find(|shape| shape.name == "Chart")
        .unwrap();
    let ShapeKind::Chart(data) = &chart.kind else {
        panic!("диаграмма должна разобраться");
    };
    assert_eq!(data.title.as_deref(), Some("Sales"));
    assert_eq!(data.categories, vec!["Q1", "Q2"]);
    assert_eq!(data.series.len(), 1);
    assert_eq!(data.series[0].values, vec![10.0, 20.0]);
}

#[test]
fn layout_and_render_produce_image_and_chart_commands() {
    let pres = doc_converter_pptx::open(package()).unwrap();
    let layout = layout_slide(&pres, 0).unwrap();
    let mut out = DisplayList::new();
    render::build(&layout, &Viewport::default(), &mut out);
    let bytes = out.to_bytes();
    let reader = DisplayList::from_bytes(&bytes).unwrap();
    let commands: Vec<_> = reader.iter().collect::<Result<_, _>>().unwrap();
    assert!(commands
        .iter()
        .any(|cmd| matches!(cmd, DrawCommand::Image { bitmap_id: 0, .. })));
    assert!(commands
        .iter()
        .any(|cmd| matches!(cmd, DrawCommand::Chart { .. })));
    assert!(commands.iter().any(|cmd| {
        matches!(cmd, DrawCommand::Text { text, .. } if reader.string(*text).contains("Hello"))
    }));
}

#[test]
fn text_export_outlines_slide_text() {
    let pres = doc_converter_pptx::open(package()).unwrap();
    let md = to_markdown(&pres);
    assert!(md.contains("## Deck 1"));
    assert!(md.contains("Hello Slides"));
}

#[test]
fn plugin_parse_and_segmented_parse_agree() {
    let bytes = package();
    let plugin = PptxPlugin;
    let via_plugin = plugin.parse(&bytes, &ZipLimits::default()).unwrap();
    assert_eq!(via_plugin.page_count(), 1);

    let mut buf = SegmentedBuffer::new();
    for chunk in bytes.chunks(7) {
        buf.append(chunk);
    }
    let via_stream = doc_converter_pptx::parse::parse_segmented(buf, ZipLimits::default()).unwrap();
    assert_eq!(via_stream.slide_count(), 1);
    assert_eq!(via_stream.slides[0].name, "Deck 1");

    let frame = via_plugin
        .build_display_list(&doc_converter_render::plugin::RenderContext {
            scale: 1.0,
            viewport_width: 1280,
            viewport_height: 720,
        })
        .unwrap();
    assert!(!frame.is_empty());
}
