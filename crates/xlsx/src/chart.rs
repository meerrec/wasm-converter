//! Диаграммы листа: разбор `xl/charts/chart*.xml` в [`ChartData`].
//!
//! Диаграмма — отдельная часть пакета; лист ссылается на неё через чертёж
//! (`xdr:graphicFrame` → `c:chart r:id`), а чертёж — через свои связи. Здесь
//! разбираются данные: вид диаграммы, заголовок, категории и серии.
//!
//! Значения берутся из кэшей (`c:numCache`/`c:strCache`): Excel их пишет, и
//! они избавляют от вычисления формул. Серия без кэша остаётся без значений —
//! книга от этого не страдает, а восстанавливать диапазоны по `c:f` — задача
//! следующего спринта.
//!
//! Поддержаны пять видов: столбцы, линии, круговая, точки и области. Прочие
//! (лепестковая, биржевая, поверхностная) не разбираются — `open` пропускает
//! такую диаграмму, как пропускает битую связь.

use doc_converter_core::xml::XmlReader;
use doc_converter_render::chart::{ChartData, ChartKind, ChartSeries};
use quick_xml::events::{BytesEnd, BytesStart, Event};

use crate::error::{Result, XlsxError};
use crate::xml::{attributes, find};

/// Разобрать часть диаграммы.
///
/// # Errors
/// [`XlsxError::Core`] — XML не разбирается; [`XlsxError::Malformed`] — вид
/// диаграммы не поддержан.
pub fn parse(bytes: &[u8], part: impl Into<String>) -> Result<ChartData> {
    let part = part.into();
    let mut parser = ChartParser::default();
    let mut reader = XmlReader::new(bytes, part.clone());
    while let Some(event) = reader.next_significant()? {
        parser.handle(event);
    }
    match parser.kind {
        Some(kind) => Ok(ChartData {
            kind,
            title: parser.title.filter(|title| !title.is_empty()),
            categories: parser.categories.unwrap_or_default(),
            series: parser.series,
        }),
        None => Err(XlsxError::malformed(&part, "unsupported chart type")),
    }
}

/// Что получает текст `c:v`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Sink {
    #[default]
    Idle,
    Title,
    Name,
    Category,
    Value,
    XValue,
}

/// Активный кэш значений.
#[derive(Debug)]
struct Cache {
    sink: Sink,
    /// Индекс текущей точки из атрибута `idx`.
    idx: usize,
    /// `ptCount` — сколько точек обещано; хвостовые пропуски добиваются пустыми.
    count: usize,
    points: Vec<Option<String>>,
}

/// Собираемая серия.
#[derive(Debug, Default)]
struct SeriesState {
    name: String,
    categories: Vec<String>,
    values: Vec<f32>,
}

#[derive(Debug, Default)]
struct ChartParser {
    kind: Option<ChartKind>,
    title: Option<String>,
    categories: Option<Vec<String>>,
    series: Vec<ChartSeries>,
    current: Option<SeriesState>,
    sink: Sink,
    text: String,
    cache: Option<Cache>,
    in_title: bool,
}

impl ChartParser {
    fn handle(&mut self, event: Event<'static>) {
        match event {
            Event::Start(element) => self.on_start(&element),
            Event::Empty(element) => self.on_empty(&element),
            Event::Text(chunk) => {
                if let Ok(text) = chunk.xml10_content() {
                    self.text.push_str(&text);
                }
            }
            Event::End(element) => self.on_end(&element),
            _ => {}
        }
    }

    fn on_start(&mut self, element: &BytesStart<'_>) {
        match element.local_name().as_ref() {
            b"barChart" | b"bar3DChart" => self.set_kind(ChartKind::Bar),
            b"lineChart" | b"line3DChart" => self.set_kind(ChartKind::Line),
            b"pieChart" | b"pie3DChart" | b"doughnutChart" => self.set_kind(ChartKind::Pie),
            b"scatterChart" | b"bubbleChart" => self.set_kind(ChartKind::Scatter),
            b"areaChart" | b"area3DChart" => self.set_kind(ChartKind::Area),
            b"title" => self.in_title = true,
            b"ser" => self.current = Some(SeriesState::default()),
            b"tx" => {
                self.sink = if self.in_title {
                    Sink::Title
                } else {
                    Sink::Name
                }
            }
            b"cat" => self.sink = Sink::Category,
            b"val" | b"yVal" => self.sink = Sink::Value,
            b"xVal" => self.sink = Sink::XValue,
            b"strCache" | b"numCache" => {
                self.cache = Some(Cache {
                    sink: self.sink,
                    idx: 0,
                    count: 0,
                    points: Vec::new(),
                });
            }
            b"ptCount" => {
                if let Some(cache) = self.cache.as_mut() {
                    cache.count = attr(element, "val")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                }
            }
            b"pt" => {
                if let Some(cache) = self.cache.as_mut() {
                    cache.idx = attr(element, "idx")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(cache.points.len());
                }
            }
            b"v" | b"t" => self.text.clear(),
            _ => {}
        }
    }

    fn on_empty(&mut self, element: &BytesStart<'_>) {
        match element.local_name().as_ref() {
            b"ptCount" => {
                if let Some(cache) = self.cache.as_mut() {
                    cache.count = attr(element, "val")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                }
            }
            b"pt" => {
                if let Some(cache) = self.cache.as_mut() {
                    cache.idx = attr(element, "idx")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(cache.points.len());
                }
            }
            b"v" | b"t" => self.text.clear(),
            _ => {}
        }
    }

    fn on_end(&mut self, element: &BytesEnd<'_>) {
        match element.local_name().as_ref() {
            b"v" => self.take_value(),
            b"t" => {
                let text = std::mem::take(&mut self.text);
                if self.in_title {
                    self.title.get_or_insert_with(String::new).push_str(&text);
                } else if let Some(series) = self.current.as_mut() {
                    series.name.push_str(&text);
                }
            }
            b"strCache" | b"numCache" => self.flush_cache(),
            b"tx" | b"cat" | b"val" | b"yVal" | b"xVal" => self.sink = Sink::Idle,
            b"title" => self.in_title = false,
            b"ser" => {
                if let Some(series) = self.current.take() {
                    self.series.push(ChartSeries {
                        name: series.name,
                        values: series.values,
                    });
                    self.categories.get_or_insert(series.categories);
                }
            }
            _ => {}
        }
    }

    fn set_kind(&mut self, kind: ChartKind) {
        // Первый вид в документе — основной: у комбинированной диаграммы
        // несколько блоков, и полноценно её всё равно не нарисовать одним видом.
        if self.kind.is_none() {
            self.kind = Some(kind);
        }
    }

    /// `c:v` вне кэша: заголовок, имя серии, одиночное значение.
    fn take_value(&mut self) {
        let text = std::mem::take(&mut self.text);
        if let Some(cache) = self.cache.as_mut() {
            let idx = cache.idx;
            if cache.points.len() <= idx {
                cache.points.resize(idx + 1, None);
            }
            cache.points[idx] = Some(text);
            return;
        }
        match self.sink {
            Sink::Title => {
                self.title.get_or_insert_with(String::new).push_str(&text);
            }
            Sink::Name => {
                if let Some(series) = self.current.as_mut() {
                    series.name = text;
                }
            }
            Sink::Category => {
                if let Some(series) = self.current.as_mut() {
                    series.categories.push(text);
                }
            }
            Sink::Value => {
                if let Some(series) = self.current.as_mut() {
                    series.values.push(text.trim().parse().unwrap_or(0.0));
                }
            }
            Sink::XValue => {
                // Координата X точечной диаграммы: пока подпись категории —
                // painter расставляет точки равномерно (см. ROADMAP).
                if let Some(series) = self.current.as_mut() {
                    series.categories.push(text);
                }
            }
            Sink::Idle => {}
        }
    }

    /// Кэш закончился: разложить точки по приёмнику.
    fn flush_cache(&mut self) {
        let Some(cache) = self.cache.take() else {
            return;
        };
        let mut points = cache.points;
        points.resize(cache.count.max(points.len()), None);
        let text: Vec<String> = points.into_iter().map(Option::unwrap_or_default).collect();

        match cache.sink {
            Sink::Title => {
                self.title
                    .get_or_insert_with(String::new)
                    .push_str(&text.join(""));
            }
            Sink::Name => {
                if let Some(series) = self.current.as_mut() {
                    series.name = text.into_iter().next().unwrap_or_default();
                }
            }
            Sink::Category | Sink::XValue => {
                if let Some(series) = self.current.as_mut() {
                    series.categories = text;
                }
            }
            Sink::Value => {
                if let Some(series) = self.current.as_mut() {
                    series.values = text
                        .iter()
                        .map(|value| value.trim().parse().unwrap_or(0.0))
                        .collect();
                }
            }
            Sink::Idle => {}
        }
    }
}

/// Значение атрибута по локальному имени; битые атрибуты — как отсутствие.
fn attr(element: &BytesStart<'_>, name: &str) -> Option<String> {
    let attrs = attributes(element, "chart").ok()?;
    find(&attrs, name).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    fn parse_ok(xml: &str) -> ChartData {
        parse(xml.as_bytes(), "xl/charts/chart1.xml").unwrap()
    }

    fn cache(kind: &str, values: &[&str]) -> String {
        let mut pts = String::new();
        for (i, value) in values.iter().enumerate() {
            let _ = write!(pts, r#"<c:pt idx="{i}"><c:v>{value}</c:v></c:pt>"#);
        }
        format!(
            r#"<c:{kind}><c:ptCount val="{}"/>{pts}</c:{kind}>"#,
            values.len()
        )
    }

    fn chart_xml(kind: &str, series: &str) -> String {
        format!(
            r#"<?xml version="1.0"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
  <c:chart>
    <c:title><c:tx><c:rich><a:p xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:r><a:t>Продажи</a:t></a:r></a:p></c:rich></c:tx></c:title>
    <c:plotArea>
      <c:{kind}>
        {series}
      </c:{kind}>
    </c:plotArea>
  </c:chart>
</c:chartSpace>"#
        )
    }

    #[test]
    fn parses_bar_chart_with_cached_values() {
        let series = format!(
            r"<c:ser>
  <c:tx><c:strRef>{}</c:strRef></c:tx>
  <c:cat><c:strRef>{}</c:strRef></c:cat>
  <c:val><c:numRef>{}</c:numRef></c:val>
</c:ser>",
            cache("strCache", &["План"]),
            cache("strCache", &["Q1", "Q2"]),
            cache("numCache", &["10", "20"]),
        );
        let chart = parse_ok(&chart_xml("barChart", &series));
        assert_eq!(chart.kind, ChartKind::Bar);
        assert_eq!(chart.title.as_deref(), Some("Продажи"));
        assert_eq!(chart.categories, vec!["Q1", "Q2"]);
        assert_eq!(chart.series.len(), 1);
        assert_eq!(chart.series[0].name, "План");
        assert_eq!(chart.series[0].values, vec![10.0, 20.0]);
    }

    #[test]
    fn recognizes_all_five_kinds() {
        let cases = [
            ("barChart", ChartKind::Bar),
            ("lineChart", ChartKind::Line),
            ("pieChart", ChartKind::Pie),
            ("scatterChart", ChartKind::Scatter),
            ("areaChart", ChartKind::Area),
        ];
        for (element, expected) in cases {
            let series = format!(
                r"<c:ser><c:val><c:numRef>{}</c:numRef></c:val></c:ser>",
                cache("numCache", &["1", "2"]),
            );
            assert_eq!(parse_ok(&chart_xml(element, &series)).kind, expected);
        }
    }

    #[test]
    fn scatter_reads_x_and_y() {
        let series = format!(
            r"<c:ser>
  <c:xVal><c:numRef>{}</c:numRef></c:xVal>
  <c:yVal><c:numRef>{}</c:numRef></c:yVal>
</c:ser>",
            cache("numCache", &["1", "2"]),
            cache("numCache", &["5", "7"]),
        );
        let chart = parse_ok(&chart_xml("scatterChart", &series));
        assert_eq!(chart.series[0].values, vec![5.0, 7.0]);
        assert_eq!(chart.categories, vec!["1", "2"]);
    }

    #[test]
    fn missing_cache_leaves_series_empty() {
        let chart = parse_ok(&chart_xml(
            "pieChart",
            "<c:ser><c:tx><c:v>Без данных</c:v></c:tx><c:val><c:numRef><c:f>Sheet1!$B$2:$B$3</c:f></c:numRef></c:val></c:ser>",
        ));
        assert_eq!(chart.series.len(), 1);
        assert_eq!(chart.series[0].name, "Без данных");
        assert!(chart.series[0].values.is_empty());
    }

    #[test]
    fn unsupported_type_is_an_error() {
        assert!(parse(
            chart_xml("radarChart", "").as_bytes(),
            "xl/charts/chart1.xml"
        )
        .is_err());
    }
}
