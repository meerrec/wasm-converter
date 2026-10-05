//! Сквозная проверка диаграмм: часть `xl/charts/chart1.xml` → модель → кадр.
//!
//! Пакет собирается в памяти: связь «лист → чертёж → диаграмма» — это три
//! части и два rels-файла, и проверять её на настоящем Excel-файле значило бы
//! прятать разрыв связи за фикстурой.

use std::io::{Cursor, Write};

use doc_converter_render::chart::{ChartData, ChartKind};
use doc_converter_render::display_list::{DisplayList, DrawCommand};
use doc_converter_xlsx::{open, paint_sheet, PaintOptions, Viewport};

const DRAWING: &str = "xl/drawings/drawing1.xml";

fn package(chart: &str) -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml"
            ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
</Types>"#;
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Target="xl/workbook.xml"
                Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"/>
</Relationships>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Лист1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
    let workbook_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Target="worksheets/sheet1.xml"
                Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"/>
</Relationships>"#;
    let sheet = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
           xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetData/>
  <drawing r:id="rId1"/>
</worksheet>"#;
    let sheet_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Target="../drawings/drawing1.xml"
                Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing"/>
</Relationships>"#;
    let drawing = r#"<?xml version="1.0" encoding="UTF-8"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"
          xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
          xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <xdr:twoCellAnchor>
    <xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
    <xdr:to><xdr:col>5</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>6</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
    <xdr:graphicFrame macro="">
      <xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Диаграмма 1"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>
      <xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm>
      <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
        <c:chart r:id="rId1"/>
      </a:graphicData></a:graphic>
    </xdr:graphicFrame>
    <xdr:clientData/>
  </xdr:twoCellAnchor>
</xdr:wsDr>"#;
    let drawing_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Target="../charts/chart1.xml"
                Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart"/>
</Relationships>"#;

    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in [
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", root_rels),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_rels),
            ("xl/worksheets/sheet1.xml", sheet),
            ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels),
            (DRAWING, drawing),
            ("xl/drawings/_rels/drawing1.xml.rels", drawing_rels),
            ("xl/charts/chart1.xml", chart),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf
}

const CHART: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
              xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <c:chart>
    <c:title><c:tx><c:rich><a:p><a:r><a:t>Итоги</a:t></a:r></a:p></c:rich></c:tx></c:title>
    <c:plotArea>
      <c:barChart>
        <c:ser>
          <c:tx><c:strRef><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>План</c:v></c:pt></c:strCache></c:strRef></c:tx>
          <c:cat><c:strRef><c:strCache><c:ptCount val="2"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strCache></c:strRef></c:cat>
          <c:val><c:numRef><c:numCache><c:ptCount val="2"/><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>20</c:v></c:pt></c:numCache></c:numRef></c:val>
        </c:ser>
      </c:barChart>
    </c:plotArea>
  </c:chart>
</c:chartSpace>"#;

#[test]
fn open_resolves_drawing_relation_to_chart_part() {
    let book = open(package(CHART)).expect("книга открывается");
    let sheet = &book.sheets()[0];
    assert_eq!(sheet.charts.len(), 1);

    let chart = &sheet.charts[0];
    assert_eq!(chart.chart.kind, ChartKind::Bar);
    assert_eq!(chart.chart.title.as_deref(), Some("Итоги"));
    assert_eq!(chart.chart.categories, vec!["Q1", "Q2"]);
    assert_eq!(chart.chart.series[0].name, "План");
    assert_eq!(chart.chart.series[0].values, vec![10.0, 20.0]);
}

#[test]
fn chart_reaches_the_frame_as_a_command() {
    let book = open(package(CHART)).expect("книга открывается");
    let sheet = &book.sheets()[0];
    let mut dl = DisplayList::new();
    paint_sheet(
        &book,
        sheet,
        Viewport::default(),
        &PaintOptions::default(),
        &mut dl,
    );

    let chart = (0..dl.len()).find_map(|i| match dl.cmd(i) {
        Some(DrawCommand::Chart { x, y, w, h, data }) => Some((*x, *y, *w, *h, *data)),
        _ => None,
    });
    let (x, y, w, h, data) = chart.expect("в кадре есть команда диаграммы");
    // Якорь B2:F7 со заголовками: 44 + 1 столбец и 20 + 1 строка.
    assert_eq!((x, y, w, h), (108.0, 40.0, 256.0, 100.0));
    let decoded = ChartData::from_blob(dl.bytes(data)).expect("блоб разбирается");
    assert_eq!(decoded.series[0].values, vec![10.0, 20.0]);
}

#[test]
fn unsupported_chart_part_is_skipped_not_fatal() {
    let radar = CHART.replace("barChart", "radarChart");
    let book = open(package(&radar)).expect("книга без поддержанной диаграммы открывается");
    assert!(book.sheets()[0].charts.is_empty());
}
