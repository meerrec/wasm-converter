//! Форматы чисел Excel: код формата превращает число в то, что видно в ячейке.
//!
//! Код состоит из секций, разделённых `;`: положительные числа, отрицательные,
//! ноль и текст. Секций может быть меньше четырёх, и тогда работает правило
//! подстановки: с одной секцией отрицательные получают минус, с двумя вторая
//! секция отвечает за отрицательные, с тремя третья — за ноль. Пустая секция
//! означает «не показывать»: `0;` прячет отрицательные.
//!
//! Разбор отделён от применения: [`NumberFormat::parse`] вызывается один раз на
//! формат, [`NumberFormat::format`] — на каждую ячейку. Форматов в книге
//! десятки, ячеек — миллионы.
//!
//! # Системы дат
//!
//! Excel унаследовал от Lotus 1-2-3 ошибку: 1900 год считается високосным, и
//! серийный номер 60 соответствует несуществующему 29.02.1900. До 61-го номера
//! отсчёт идёт от 31.12.1899, от 60-го — от 30.12.1899, чтобы настоящие даты
//! совпали. День недели считается прямо от номера: так он совпадает с Excel и
//! до, и после «фантомного» дня. В системе 1904 года (книги для Mac) точка
//! отсчёта — 01.01.1904, и бага там нет.
//!
//! # Чего нет
//!
//! Дроби (`# ?/?`), узор `*x` и цвета (`[Red]`) на текст не влияют: цвет — дело
//! рендера, узор в тексте не виден, а дроби падают в общий формат. Имена месяцев
//! и дней английские: язык формата зависит от локали, которой у парсера нет.
//! `TODO (Фаза 5)`: дроби и локализация имён.

use std::fmt::Write as _;

use chrono::{Datelike, Days, NaiveDate};

/// Код формата по умолчанию.
pub const GENERAL: &str = "General";

/// Код встроенного формата по его `numFmtId` (ECMA-376, §18.8.30).
///
/// Идентификаторы 14–22 и 45–47 зависят от локали; здесь вариант `en-US` — тот,
/// что записан в спецификации.
#[must_use]
pub fn builtin(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "$#,##0_);($#,##0)",
        6 => "$#,##0_);[Red]($#,##0)",
        7 => "$#,##0.00_);($#,##0.00)",
        8 => "$#,##0.00_);[Red]($#,##0.00)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "m/d/yyyy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yyyy h:mm",
        37 => "#,##0_);(#,##0)",
        38 => "#,##0_);[Red](#,##0)",
        39 => "#,##0.00_);(#,##0.00)",
        40 => "#,##0.00_);[Red](#,##0.00)",
        41 => r#"_(* #,##0_);_(* (#,##0);_(* "-"_);_(@_)"#,
        42 => r#"_($* #,##0_);_($* (#,##0);_($* "-"_);_(@_)"#,
        43 => r#"_(* #,##0.00_);_(* (#,##0.00);_(* "-"??_);_(@_)"#,
        44 => r#"_($* #,##0.00_);_($* (#,##0.00);_($* "-"??_);_(@_)"#,
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// Разобранный код формата.
#[derive(Debug, Clone)]
pub struct NumberFormat {
    sections: Vec<Section>,
}

impl NumberFormat {
    /// Разобрать код формата. Пустой код — это `General`.
    #[must_use]
    pub fn parse(code: &str) -> Self {
        if code.is_empty() {
            return Self {
                sections: vec![Section::General],
            };
        }
        let sections = code
            .split(';')
            .map(|part| {
                if part.is_empty() {
                    // Пустая секция — «не показывать», а не «показать как есть».
                    Section::Hidden
                } else {
                    Section::parse(part)
                }
            })
            .collect();
        Self { sections }
    }

    /// Отформатировать число.
    #[must_use]
    pub fn format(&self, value: f64, date1904: bool) -> String {
        let negative = value < 0.0;
        let index = self.number_section_index(value);
        let Some(section) = self.sections.get(index) else {
            return render_general(value);
        };

        // Знак отдаём секции: числовой шаблон берёт модуль сам, а дата из
        // отрицательного номера не получается — Excel показывает решётки.
        let mut text = section.render(value, date1904);
        if negative && index == 0 && matches!(section, Section::Number(_)) && !text.starts_with('-')
        {
            // С одной секцией минус дописывает сам формат. Числовой шаблон
            // печатает модуль, поэтому знак нужен; `General` выводит знак сам,
            // а дата из отрицательного номера — это решётки, без минуса.
            text.insert(0, '-');
        }
        text
    }

    /// Отформатировать текст ячейки: секция текста, если она есть, иначе текст
    /// как он записан.
    #[must_use]
    pub fn format_text(&self, text: &str) -> String {
        let section = self.sections.get(3).or_else(|| self.sections.first());
        match section {
            Some(section @ Section::Text(_)) => section.render_text(text),
            _ => text.to_owned(),
        }
    }

    /// Показывает ли формат дату или время.
    #[must_use]
    pub fn is_date(&self) -> bool {
        self.sections
            .iter()
            .any(|section| matches!(section, Section::DateTime(_)))
    }

    /// Номер секции для числа по правилам подстановки.
    fn number_section_index(&self, value: f64) -> usize {
        let count = self.sections.len();
        if value < 0.0 {
            usize::from(count >= 2)
        } else if value == 0.0 && count >= 3 {
            2
        } else {
            0
        }
    }
}

/// Разобрать код и отформатировать значение — короткий путь для одной ячейки.
///
/// Для многих ячеек выгоднее разобрать код один раз в [`NumberFormat`].
#[must_use]
pub fn format(value: f64, code: &str, date1904: bool) -> String {
    NumberFormat::parse(code).format(value, date1904)
}

/// Секция кода формата.
#[derive(Debug, Clone)]
enum Section {
    /// `General` — значение как есть.
    General,
    /// Значение не показывать (пустая секция).
    Hidden,
    /// Числовой шаблон.
    Number(Box<Mask>),
    /// Дата и время: части и литералы между ними.
    DateTime(Vec<Token>),
    /// Текст с подстановкой `@`.
    Text(Vec<Token>),
}

impl Section {
    /// Разобрать одну секцию кода.
    fn parse(code: &str) -> Self {
        let mut tokens = tokenize(code);
        resolve_minutes(&mut tokens);

        if is_general(&tokens) {
            return Self::General;
        }

        let has_date = tokens.iter().any(|token| matches!(token, Token::Date(_)));
        let has_digit = tokens.iter().any(|token| matches!(token, Token::Digit(_)));
        let has_text = tokens.iter().any(|token| matches!(token, Token::Text));

        if has_date && !has_digit {
            Self::DateTime(tokens)
        } else if has_text && !has_digit {
            Self::Text(tokens)
        } else {
            // Числа и всё остальное — в том числе секция из одних литералов,
            // где значение не показывается, а литералы печатаются.
            Self::Number(Box::new(Mask::build(&tokens)))
        }
    }

    /// Отформатировать число этой секцией.
    fn render(&self, value: f64, date1904: bool) -> String {
        match self {
            Self::General => render_general(value),
            Self::Hidden => String::new(),
            Self::Number(mask) => mask.render(value),
            Self::DateTime(tokens) => render_datetime(tokens, value, date1904),
            Self::Text(tokens) => Self::Text(tokens.clone()).render_text(""),
        }
    }

    /// Отформатировать текст этой секцией.
    fn render_text(&self, text: &str) -> String {
        let Self::Text(tokens) = self else {
            return text.to_owned();
        };
        let mut out = String::new();
        for token in tokens {
            match token {
                Token::Literal(literal) => out.push_str(literal),
                Token::Text => out.push_str(text),
                Token::Percent => out.push('%'),
                _ => {}
            }
        }
        out
    }
}

/// Токен кода формата.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// Готовый текст: в кавычках, после `\`, символ валюты.
    Literal(String),
    /// `0`, `#` или `?`.
    Digit(Digit),
    /// Десятичная точка.
    Point,
    /// Запятая: разделитель разрядов или деление на 1000.
    Comma,
    /// `%`.
    Percent,
    /// `@` — подстановка текста.
    Text,
    /// Часть даты или времени.
    Date(DatePart),
    /// `E+` или `E-`; `true` — знак экспоненты обязателен.
    Exponent(bool),
}

/// Разряд числа в шаблоне.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Digit {
    /// `0` — разряд показывается всегда.
    Zero,
    /// `#` — незначащий ноль не показывается.
    Hash,
    /// `?` — вместо незначащего нуля пробел, чтобы разряды сошлись.
    Question,
}

/// Часть даты или времени.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatePart {
    /// `y`..`yyyy`.
    Year(u8),
    /// `m`..`mmmmm`.
    Month(u8),
    /// `d`..`dddd`.
    Day(u8),
    /// `h`, `hh`.
    Hour(u8),
    /// `m` после часов или перед секундами.
    Minute(u8),
    /// `s`, `ss`.
    Second(u8),
    /// Доли секунды: сколько знаков после точки.
    Subsecond(u8),
    /// `AM/PM` (`true`) или `A/P` (`false`).
    AmPm(bool),
    /// Прошедшее время: `[h]`, `[m]`, `[s]` — без перехода через сутки.
    Elapsed(Unit),
}

/// Единица прошедшего времени.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Hour,
    Minute,
    Second,
}

/// Числовой шаблон: всё, что нужно, чтобы напечатать число без повторного
/// разбора кода.
///
/// Целая часть хранится токенами: кроме разрядов в ней бывают литералы
/// (`000\-00\-00` для телефонов), и печатать их нужно на своём месте.
#[derive(Debug, Clone, Default)]
struct Mask {
    prefix: Vec<Token>,
    suffix: Vec<Token>,
    /// Целая часть: разряды, литералы и запятые в порядке записи.
    integer: Vec<Token>,
    /// Обязательных разрядов в каждом непрерывном ряду цифр целой части.
    integer_runs: Vec<usize>,
    /// Дробная часть: разряды и литералы в порядке записи.
    fraction: Vec<Token>,
    /// Обязательных разрядов целой части всего.
    min_integer: usize,
    /// Дополнять целую часть пробелами (`?`).
    align_integer: bool,
    /// Обязательных разрядов дробной части.
    min_fraction: usize,
    /// Разрядов дробной части в шаблоне.
    max_fraction: usize,
    /// Дополнять дробную часть пробелами (`?`).
    align_fraction: bool,
    /// Группировать разряды по три.
    grouping: bool,
    /// Сколько раз делить на 1000 (запятые после последнего разряда).
    scale: u32,
    /// Сколько раз умножить на 100 (`%`).
    percents: u32,
    /// Экспонента: `Some(true)` — знак обязателен.
    exponent: Option<bool>,
    /// Разрядов в экспоненте.
    exponent_digits: usize,
}

impl Mask {
    /// Собрать шаблон из токенов секции.
    fn build(tokens: &[Token]) -> Self {
        let first = tokens
            .iter()
            .position(|token| matches!(token, Token::Digit(_)));
        let last = tokens
            .iter()
            .rposition(|token| matches!(token, Token::Digit(_)));

        let mut mask = Self::default();
        let (Some(first), Some(last)) = (first, last) else {
            // Разрядов нет вовсе: печатаем только литералы.
            mask.prefix = tokens.to_vec();
            return mask;
        };

        mask.prefix = tokens[..first].to_vec();
        mask.suffix = tokens[last + 1..].to_vec();
        mask.percents = u32::try_from(
            tokens
                .iter()
                .filter(|token| matches!(token, Token::Percent))
                .count(),
        )
        .unwrap_or(u32::MAX);
        // Запятые после последнего разряда делят значение на тысячу.
        mask.scale = u32::try_from(
            tokens[last + 1..]
                .iter()
                .filter(|token| matches!(token, Token::Comma))
                .count(),
        )
        .unwrap_or(u32::MAX);

        let body = &tokens[first..=last];
        // Разряды экспоненты — не разряды дробной части.
        let (mantissa, exponent) = match body
            .iter()
            .position(|token| matches!(token, Token::Exponent(_)))
        {
            Some(at) => (&body[..at], Some(at)),
            None => (body, None),
        };
        if let Some(at) = exponent {
            mask.exponent = Some(matches!(body[at], Token::Exponent(true)));
            mask.exponent_digits = body[at + 1..]
                .iter()
                .filter(|token| matches!(token, Token::Digit(_)))
                .count();
        }

        let (integer, fraction) = match mantissa.iter().position(|t| matches!(t, Token::Point)) {
            Some(at) => (&mantissa[..at], &mantissa[at + 1..]),
            None => (mantissa, &[][..]),
        };
        mask.integer = integer.to_vec();
        mask.fraction = fraction.to_vec();
        mask.integer_runs = digit_runs(integer);
        mask.min_integer = mask.integer_runs.iter().sum();
        mask.align_integer = integer
            .iter()
            .any(|token| matches!(token, Token::Digit(Digit::Question)));
        mask.grouping = integer.windows(2).any(|pair| {
            matches!(
                (&pair[0], &pair[1]),
                (Token::Comma, Token::Digit(_)) | (Token::Digit(_), Token::Comma)
            )
        });
        mask.min_fraction = fraction
            .iter()
            .filter(|token| matches!(token, Token::Digit(Digit::Zero)))
            .count();
        mask.max_fraction = fraction
            .iter()
            .filter(|token| matches!(token, Token::Digit(_)))
            .count();
        mask.align_fraction = fraction
            .iter()
            .any(|token| matches!(token, Token::Digit(Digit::Question)));
        mask
    }

    /// Напечатать число по шаблону.
    fn render(&self, value: f64) -> String {
        let mut value = value.abs();
        for _ in 0..self.percents {
            value *= 100.0;
        }
        for _ in 0..self.scale {
            value /= 1000.0;
        }

        let mut out = String::new();
        for token in &self.prefix {
            push_token(&mut out, token);
        }
        if let Some(required) = self.exponent {
            self.push_exponent(&mut out, value, required);
        } else {
            self.push_plain(&mut out, value);
        }
        for token in &self.suffix {
            push_token(&mut out, token);
        }
        out
    }

    /// Обычная запись числа.
    fn push_plain(&self, out: &mut String, value: f64) {
        let rounded = round_half_up(value, self.max_fraction);
        let text = format!("{:.*}", self.max_fraction, rounded);
        let (integer, fraction) = text.split_once('.').unwrap_or((text.as_str(), ""));

        self.push_integer(out, integer);

        let fraction = trim_fraction(
            fraction,
            self.min_fraction,
            self.max_fraction,
            self.align_fraction,
        );
        if !fraction.is_empty() {
            out.push('.');
            out.push_str(&fraction);
        }
    }

    /// Напечатать целую часть: разряды, литералы между ними и разделители.
    fn push_integer(&self, out: &mut String, digits: &str) {
        let mut digits = digits.to_owned();
        while digits.len() < self.min_integer {
            digits.insert(0, '0');
        }
        if self.align_integer {
            // Место недостающих разрядов занимают пробелы.
            let padding = self
                .integer_runs
                .iter()
                .sum::<usize>()
                .saturating_sub(digits.len());
            for _ in 0..padding {
                out.push(' ');
            }
        }

        let plain = self
            .integer
            .iter()
            .all(|token| matches!(token, Token::Digit(_) | Token::Comma));
        if plain {
            // Необязательные ведущие нули не показываем.
            let keep = digits.trim_start_matches('0').len().max(self.min_integer);
            let digits = &digits[digits.len() - keep..];
            if self.grouping {
                push_grouped(out, digits);
            } else {
                out.push_str(digits);
            }
            return;
        }

        // Литералы внутри целой части: разряды идут по рядам, а всё, что сверх
        // минимума, достаётся первому ряду — как в `000\-00\-00`.
        let mut rest = digits.as_str();
        let extra = rest.len().saturating_sub(self.min_integer);
        let mut runs = self.integer_runs.iter().copied();
        let mut allowed = runs.next().unwrap_or(0) + extra;
        let mut used = 0;
        for token in &self.integer {
            match token {
                Token::Digit(_) => {
                    if used < allowed {
                        if let Some(ch) = rest.chars().next() {
                            out.push(ch);
                            rest = &rest[ch.len_utf8()..];
                            used += 1;
                        }
                    }
                }
                Token::Literal(text) => {
                    out.push_str(text);
                    used = 0;
                    allowed = runs.next().unwrap_or(0);
                }
                _ => {}
            }
        }
    }

    /// Запись в научном виде: `1.23E+05`.
    fn push_exponent(&self, out: &mut String, value: f64, required_sign: bool) {
        #[allow(clippy::cast_possible_truncation)]
        // Порядок double лежит в −324..=308, в i32 влезает с запасом.
        let mut exponent = if value == 0.0 {
            0
        } else {
            value.abs().log10().floor() as i32
        };
        let mut mantissa = value / 10f64.powi(exponent);
        // Округление мантиссы могло дойти до десяти — тогда порядок сдвигается.
        if round_half_up(mantissa, self.max_fraction) >= 10.0 {
            mantissa /= 10.0;
            exponent += 1;
        }

        let digits = self.max_fraction;
        let rounded = round_half_up(mantissa, digits);
        let text = format!("{rounded:.digits$}");
        let (integer, fraction) = text.split_once('.').unwrap_or((text.as_str(), ""));
        out.push_str(integer);
        let fraction = trim_fraction(fraction, self.min_fraction, self.max_fraction, false);
        if !fraction.is_empty() {
            out.push('.');
            out.push_str(&fraction);
        }

        out.push('E');
        if exponent < 0 {
            out.push('-');
        } else if required_sign {
            out.push('+');
        }
        let width = self.exponent_digits.max(1);
        let exponent = exponent.unsigned_abs().to_string();
        for _ in 0..width.saturating_sub(exponent.len()) {
            out.push('0');
        }
        out.push_str(&exponent);
    }
}

/// Обязательные разряды каждого непрерывного ряда цифр.
///
/// Ряд из одних `#` даёт ноль обязательных разрядов, но сам ряд существует:
/// иначе литералы после него потеряли бы своё место.
fn digit_runs(tokens: &[Token]) -> Vec<usize> {
    let mut runs = Vec::new();
    let mut current: Option<usize> = None;
    for token in tokens {
        match token {
            Token::Digit(Digit::Zero) => *current.get_or_insert(0) += 1,
            Token::Digit(_) => {
                current.get_or_insert(0);
            }
            _ => {
                if let Some(run) = current.take() {
                    runs.push(run);
                }
            }
        }
    }
    if let Some(run) = current {
        runs.push(run);
    }
    runs
}

/// Напечатать токен, не относящийся к разрядам.
fn push_token(out: &mut String, token: &Token) {
    match token {
        Token::Literal(literal) => out.push_str(literal),
        Token::Percent => out.push('%'),
        // Остальное в этой позиции не печатается.
        _ => {}
    }
}

/// Разбить целую часть на разряды по три.
#[allow(clippy::manual_is_multiple_of)]
// `is_multiple_of` появился только в Rust 1.87, а MSRV проекта — 1.82.
fn push_grouped(out: &mut String, integer: &str) {
    let digits: Vec<char> = integer.chars().collect();
    for (index, ch) in digits.iter().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(*ch);
    }
}

/// Оставить в дробной части значащие разряды.
fn trim_fraction(fraction: &str, min: usize, max: usize, align: bool) -> String {
    let mut kept = fraction.len().min(max);
    while kept > min && fraction.as_bytes().get(kept - 1) == Some(&b'0') {
        kept -= 1;
    }
    let mut out = fraction[..kept].to_owned();
    if align {
        for _ in kept..max {
            out.push(' ');
        }
    }
    out
}

/// Округлить половину вверх — как Excel, а не как двоичная арифметика.
fn round_half_up(value: f64, digits: usize) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let factor = 10f64.powi(i32::try_from(digits).unwrap_or(i32::MAX));
    if !factor.is_finite() || factor == 0.0 {
        return value;
    }
    (value * factor + 0.5).floor() / factor
}

/// Разобрать код на токены.
fn tokenize(code: &str) -> Vec<Token> {
    let chars: Vec<char> = code.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        match ch {
            '"' => {
                let (literal, next) = read_quoted(&chars, index);
                tokens.push(Token::Literal(literal));
                index = next;
            }
            '\\' => {
                if let Some(next) = chars.get(index + 1) {
                    tokens.push(Token::Literal(next.to_string()));
                }
                index += 2;
            }
            '_' => {
                // Пропуск ширины символа: в тексте это место.
                tokens.push(Token::Literal(" ".to_owned()));
                index += 2;
            }
            '*' => index += 2,
            '[' => {
                let (token, next) = read_bracket(&chars, index);
                if let Some(token) = token {
                    tokens.push(token);
                }
                index = next;
            }
            '0' | '#' | '?' => {
                tokens.push(Token::Digit(match ch {
                    '0' => Digit::Zero,
                    '#' => Digit::Hash,
                    _ => Digit::Question,
                }));
                index += 1;
            }
            '.' => {
                // Точка сразу после секунд — это доли секунды, а не разряды
                // числа: `hh:mm:ss.00`.
                if matches!(tokens.last(), Some(Token::Date(DatePart::Second(_)))) {
                    let run = chars[index + 1..].iter().take_while(|&&c| c == '0').count();
                    let count = u8::try_from(run).unwrap_or(u8::MAX);
                    tokens.push(Token::Date(DatePart::Subsecond(count)));
                    index += 1 + run;
                } else {
                    tokens.push(Token::Point);
                    index += 1;
                }
            }
            ',' => {
                tokens.push(Token::Comma);
                index += 1;
            }
            '%' => {
                tokens.push(Token::Percent);
                index += 1;
            }
            '@' => {
                tokens.push(Token::Text);
                index += 1;
            }
            'e' | 'E' if matches!(chars.get(index + 1), Some('+' | '-')) => {
                tokens.push(Token::Exponent(chars[index + 1] == '+'));
                index += 2;
            }
            _ => {
                let (token, next) = read_word(&chars, index);
                tokens.push(token);
                index = next;
            }
        }
    }

    tokens
}

/// Прочитать текст в кавычках; `""` внутри — это кавычка.
fn read_quoted(chars: &[char], start: usize) -> (String, usize) {
    let mut literal = String::new();
    let mut index = start + 1;
    while index < chars.len() {
        if chars[index] == '"' {
            if chars.get(index + 1) == Some(&'"') {
                literal.push('"');
                index += 2;
                continue;
            }
            index += 1;
            break;
        }
        literal.push(chars[index]);
        index += 1;
    }
    (literal, index)
}

/// Прочитать `[...]`: прошедшее время, символ валюты или служебная пометка.
fn read_bracket(chars: &[char], start: usize) -> (Option<Token>, usize) {
    let mut index = start + 1;
    let mut content = String::new();
    while index < chars.len() && chars[index] != ']' {
        content.push(chars[index]);
        index += 1;
    }
    let next = (index + 1).min(chars.len());

    let token = match content.as_str() {
        "h" | "hh" => Some(Token::Date(DatePart::Elapsed(Unit::Hour))),
        "m" | "mm" => Some(Token::Date(DatePart::Elapsed(Unit::Minute))),
        "s" | "ss" => Some(Token::Date(DatePart::Elapsed(Unit::Second))),
        other if other.starts_with('$') => {
            // `[$€-407]` — символ валюты и код локали.
            let symbol = other[1..].split('-').next().unwrap_or("");
            Some(Token::Literal(symbol.to_owned()))
        }
        // Цвет, условие, код локали: на текст не влияют.
        _ => None,
    };
    (token, next)
}

/// Прочитать слово: часть даты, `AM/PM` или текстовый литерал.
fn read_word(chars: &[char], start: usize) -> (Token, usize) {
    let rest: String = chars[start..].iter().collect();
    let upper = rest.to_ascii_uppercase();
    if upper.starts_with("GENERAL") {
        return (Token::Literal("General".to_owned()), start + 7);
    }
    for (pattern, short) in [("AM/PM", false), ("A/P", true)] {
        if upper.starts_with(pattern) {
            return (Token::Date(DatePart::AmPm(short)), start + pattern.len());
        }
    }

    let ch = chars[start];
    if ch == '.' {
        return (Token::Point, start + 1);
    }
    let run = chars[start..].iter().take_while(|&&c| c == ch).count();
    let count = u8::try_from(run).unwrap_or(u8::MAX);
    let part = match ch.to_ascii_lowercase() {
        'y' => Some(DatePart::Year(count.min(4))),
        'm' => Some(DatePart::Month(count.min(5))),
        'd' => Some(DatePart::Day(count.min(4))),
        'h' => Some(DatePart::Hour(count.min(2))),
        's' => Some(DatePart::Second(count.min(2))),
        _ => None,
    };
    match part {
        Some(part) => (Token::Date(part), start + run),
        None => (Token::Literal(ch.to_string()), start + 1),
    }
}

/// `m` — это месяц или минуты? Минуты, если рядом часы или секунды.
fn resolve_minutes(tokens: &mut [Token]) {
    let dates: Vec<(usize, DatePart)> = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| match token {
            Token::Date(part) => Some((index, *part)),
            _ => None,
        })
        .collect();

    for (position, (index, part)) in dates.iter().enumerate() {
        let DatePart::Month(count) = part else {
            continue;
        };
        let before = position
            .checked_sub(1)
            .and_then(|previous| dates.get(previous))
            .map(|(_, part)| *part);
        let after = dates.get(position + 1).map(|(_, part)| *part);
        let minutes = matches!(before, Some(DatePart::Hour(_) | DatePart::Minute(_)))
            || matches!(after, Some(DatePart::Second(_)));
        if minutes {
            tokens[*index] = Token::Date(DatePart::Minute(*count));
        }
    }
}

/// Код — это `General` (с точностью до регистра и пробелов)?
fn is_general(tokens: &[Token]) -> bool {
    if tokens.is_empty() {
        return true;
    }
    let mut literal = String::new();
    for token in tokens {
        match token {
            Token::Literal(text) => literal.push_str(text),
            _ => return false,
        }
    }
    literal.trim().eq_ignore_ascii_case("general")
}

/// Дата и время, полученные из серийного номера.
#[derive(Debug, Clone, Copy)]
struct DateTime {
    year: i32,
    month: u32,
    /// День месяца; 0 — «нулевой день» системы 1900.
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    nanos: u32,
    /// 0 — воскресенье, как в `WEEKDAY` по умолчанию.
    weekday: u32,
}

impl DateTime {
    /// Перевести серийный номер в дату и время.
    ///
    /// `None` — номер отрицательный или выходит за пределы календаря: показать
    /// такую дату нечем.
    fn from_serial(serial: f64, date1904: bool, subsecond: u8) -> Option<Self> {
        if !serial.is_finite() || serial < 0.0 {
            return None;
        }
        // Считаем в «тиках» — 1/10^subsecond секунды, чтобы округление до
        // показанной точности само переносило лишние доли в минуты и часы.
        let factor = 10f64.powi(i32::from(subsecond));
        let ticks = (serial * 86_400.0 * factor).round();
        if !ticks.is_finite() || ticks > 9e15 {
            // Дальше точности double не хватает, а такие номера датами не бывают.
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        // Значение проверено выше и лежит в пределах точности double.
        let ticks = ticks as i64;
        #[allow(clippy::cast_possible_truncation)]
        // Множитель не больше 10⁹, в i64 влезает.
        let per_second = factor as i64;

        let seconds = ticks / per_second;
        let sub_ticks = ticks % per_second;
        let days = seconds / 86_400;
        let within = seconds % 86_400;

        #[allow(clippy::cast_sign_loss)]
        // Значения ограничены сутками, приведение к u32 без потерь.
        let (hour, minute, second) = (
            (within / 3600) as u32,
            ((within % 3600) / 60) as u32,
            (within % 60) as u32,
        );
        let nanos = if per_second > 0 {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            // Доля секунды лежит в 0..1, до 10⁹ наносекунд: точности хватает.
            let nanos = (sub_ticks as f64 / per_second as f64 * 1e9) as u32;
            nanos
        } else {
            0
        };

        let (year, month, day, weekday) = civil_from_days(days, date1904)?;
        Some(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            nanos,
            weekday,
        })
    }
}

/// Календарная дата и день недели по числу суток от точки отсчёта.
fn civil_from_days(days: i64, date1904: bool) -> Option<(i32, u32, u32, u32)> {
    if date1904 {
        // Система 1904: точка отсчёта — 01.01.1904, «фантомного» дня нет.
        let base = NaiveDate::from_ymd_opt(1904, 1, 1)?;
        let date = base.checked_add_days(Days::new(u64::try_from(days).ok()?))?;
        return Some((
            date.year(),
            date.month(),
            date.day(),
            u32::try_from((days + 5).rem_euclid(7)).ok()?,
        ));
    }

    let weekday = u32::try_from((days + 6).rem_euclid(7)).ok()?;
    if days == 0 {
        // Нулевой номер Excel показывает как «00.01.1900».
        return Some((1900, 1, 0, weekday));
    }
    if days == 60 {
        // 29.02.1900 не существовало: день вставлен ради совместимости с Lotus.
        return Some((1900, 2, 29, weekday));
    }
    let base = if days < 60 {
        NaiveDate::from_ymd_opt(1899, 12, 31)?
    } else {
        NaiveDate::from_ymd_opt(1899, 12, 30)?
    };
    let date = base.checked_add_days(Days::new(u64::try_from(days).ok()?))?;
    Some((date.year(), date.month(), date.day(), weekday))
}

/// Напечатать дату и время по частям шаблона.
fn render_datetime(tokens: &[Token], value: f64, date1904: bool) -> String {
    let parts: Vec<DatePart> = tokens
        .iter()
        .filter_map(|token| match token {
            Token::Date(part) => Some(*part),
            _ => None,
        })
        .collect();
    let subsecond = parts
        .iter()
        .filter_map(|part| match part {
            DatePart::Subsecond(count) => Some(*count),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let Some(datetime) = DateTime::from_serial(value, date1904, subsecond) else {
        // Отрицательный номер датой не является — Excel показывает решётки.
        return "#####".to_owned();
    };
    let twelve_hour = parts.iter().any(|part| matches!(part, DatePart::AmPm(_)));

    let mut out = String::new();
    for token in tokens {
        match token {
            Token::Date(part) => push_date_part(&mut out, *part, &datetime, value, twelve_hour),
            Token::Literal(text) => out.push_str(text),
            // Точка и запятая в маске даты — разделители, а не знак дробной
            // части и не масштаб: `dd.mm.yyyy` без этого печаталось слитно.
            Token::Point => out.push('.'),
            Token::Comma => out.push(','),
            _ => {}
        }
    }
    out
}

/// Напечатать одну часть даты или времени.
fn push_date_part(
    out: &mut String,
    part: DatePart,
    datetime: &DateTime,
    serial: f64,
    twelve_hour: bool,
) {
    match part {
        DatePart::Year(count) => match count {
            1 => out.push_str(&datetime.year.to_string()),
            2 => {
                let _ = write!(out, "{:02}", datetime.year % 100);
            }
            _ => {
                let _ = write!(out, "{:04}", datetime.year);
            }
        },
        DatePart::Month(count) => match count {
            1 => out.push_str(&datetime.month.to_string()),
            2 => {
                let _ = write!(out, "{:02}", datetime.month);
            }
            3 => out.push_str(MONTHS_SHORT[month_index(datetime.month)]),
            4 => out.push_str(MONTHS[month_index(datetime.month)]),
            _ => out.push_str(&MONTHS[month_index(datetime.month)][..1]),
        },
        DatePart::Day(count) => match count {
            1 => out.push_str(&datetime.day.to_string()),
            2 => {
                let _ = write!(out, "{:02}", datetime.day);
            }
            3 => out.push_str(WEEKDAYS_SHORT[weekday_index(datetime.weekday)]),
            _ => out.push_str(WEEKDAYS[weekday_index(datetime.weekday)]),
        },
        DatePart::Hour(count) => {
            let hour = if twelve_hour {
                let hour = datetime.hour % 12;
                if hour == 0 {
                    12
                } else {
                    hour
                }
            } else {
                datetime.hour
            };
            push_padded(out, hour, count);
        }
        DatePart::Minute(count) => push_padded(out, datetime.minute, count),
        DatePart::Second(count) => push_padded(out, datetime.second, count),
        DatePart::Subsecond(count) => {
            out.push('.');
            let digits = format!("{:09}", datetime.nanos);
            out.push_str(&digits[..usize::from(count).min(9)]);
        }
        DatePart::AmPm(short) => {
            let morning = datetime.hour < 12;
            out.push_str(match (short, morning) {
                (true, true) => "A",
                (true, false) => "P",
                (false, true) => "AM",
                (false, false) => "PM",
            });
        }
        DatePart::Elapsed(unit) => {
            // Прошедшее время считается от самого номера, без календаря.
            let total = match unit {
                Unit::Hour => serial * 24.0,
                Unit::Minute => serial * 1440.0,
                Unit::Second => serial * 86_400.0,
            };
            #[allow(clippy::cast_possible_truncation)]
            // Значение ограничено разумным форматом ячейки.
            let total = total.trunc() as i64;
            out.push_str(&total.to_string());
        }
    }
}

/// Напечатать число с ведущим нулём, если шаблон просит два разряда.
fn push_padded(out: &mut String, value: u32, count: u8) {
    if count >= 2 {
        let _ = write!(out, "{value:02}");
    } else {
        out.push_str(&value.to_string());
    }
}

/// Месяц 1..=12 → индекс в массиве имён.
fn month_index(month: u32) -> usize {
    usize::try_from(month.saturating_sub(1).min(11)).unwrap_or(0)
}

/// День недели 0..=6 (0 — воскресенье) → индекс в массиве имён.
fn weekday_index(weekday: u32) -> usize {
    usize::try_from(weekday.min(6)).unwrap_or(0)
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

const WEEKDAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// Значение в общем формате — так Excel показывает числа без шаблона.
fn render_general(value: f64) -> String {
    if value.is_nan() || value.is_infinite() {
        return "#NUM!".to_owned();
    }
    let abs = value.abs();
    if abs != 0.0 && (abs >= 1e11 || abs < 1e-10) {
        return scientific_general(value);
    }
    if value.fract() == 0.0 && abs < 1e15 {
        #[allow(clippy::cast_possible_truncation)]
        // Проверка выше гарантирует, что значение влезает в i64.
        return format!("{}", value as i64);
    }
    let text = format!("{value:.10}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Общий формат для очень больших и очень малых чисел: `1.23457E+15`.
fn scientific_general(value: f64) -> String {
    let text = format!("{value:.5E}");
    let Some((mantissa, exponent)) = text.split_once('E') else {
        return text;
    };
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    let (sign, digits) = match exponent.strip_prefix('-') {
        Some(digits) => ('-', digits),
        None => ('+', exponent),
    };
    format!("{mantissa}E{sign}{digits}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(value: f64, code: &str) -> String {
        format(value, code, false)
    }

    #[test]
    fn builtin_codes_are_known() {
        assert_eq!(builtin(0), Some("General"));
        assert_eq!(builtin(14), Some("m/d/yyyy"));
        assert_eq!(builtin(49), Some("@"));
        assert_eq!(builtin(23), None);
        assert_eq!(builtin(200), None);
    }

    #[test]
    fn general_format() {
        assert_eq!(render(0.0, "General"), "0");
        assert_eq!(render(42.0, "General"), "42");
        assert_eq!(render(-7.0, "General"), "-7");
        assert_eq!(render(1.5, "General"), "1.5");
        assert_eq!(render(0.1 + 0.2, "General"), "0.3");
        assert_eq!(render(1e12, "General"), "1E+12");
        assert_eq!(render(f64::NAN, "General"), "#NUM!");
    }

    #[test]
    fn plain_numbers() {
        assert_eq!(render(1.0, "0"), "1");
        assert_eq!(render(1.5, "0"), "2");
        assert_eq!(render(1.4, "0"), "1");
        assert_eq!(render(1.5, "0.00"), "1.50");
        assert_eq!(render(0.5, "0.00"), "0.50");
        assert_eq!(render(0.5, "#.##"), ".5");
        assert_eq!(render(0.0, "0.00"), "0.00");
    }

    #[test]
    fn rounding_is_half_up_like_excel() {
        assert_eq!(render(2.5, "0"), "3");
        assert_eq!(render(0.125, "0.00"), "0.13");
        assert_eq!(render(-2.5, "0"), "-3");
    }

    #[test]
    fn grouping_and_scaling() {
        assert_eq!(render(1_234_567.0, "#,##0"), "1,234,567");
        assert_eq!(render(1234.5, "#,##0.00"), "1,234.50");
        assert_eq!(render(999.0, "#,##0"), "999");
        // Запятая в конце делит на тысячу.
        assert_eq!(render(1_234_567.0, "#,##0,"), "1,235");
        assert_eq!(render(1_234_567.0, "0,,"), "1");
    }

    #[test]
    fn percents_and_literals() {
        assert_eq!(render(0.25, "0%"), "25%");
        assert_eq!(render(0.1234, "0.00%"), "12.34%");
        assert_eq!(render(1234.5, "\"₽\" #,##0.00"), "₽ 1,234.50");
        assert_eq!(render(1234.5, "[$€-407]#,##0.00"), "€1,234.50");
        assert_eq!(render(-5.0, "0.00;[Red](0.00)"), "(5.00)");
    }

    #[test]
    fn sections_pick_by_sign() {
        assert_eq!(render(5.0, "0.00;(0.00);—"), "5.00");
        assert_eq!(render(-5.0, "0.00;(0.00);—"), "(5.00)");
        assert_eq!(render(0.0, "0.00;(0.00);—"), "—");
        // Одна секция: минус дописывается сам.
        assert_eq!(render(-5.0, "0.00"), "-5.00");
        // Пустая секция прячет значение.
        assert_eq!(render(-5.0, "0.00;"), "");
        assert_eq!(render(5.0, ";;;"), "");
    }

    #[test]
    fn exponential_format() {
        assert_eq!(render(123_456.0, "0.00E+00"), "1.23E+05");
        assert_eq!(render(0.0012, "0.00E+00"), "1.20E-03");
        assert_eq!(render(1234.0, "##0.0E+0"), "1.2E+3");
    }

    #[test]
    fn text_section() {
        let format = NumberFormat::parse("0.00;@");
        assert_eq!(format.format_text("привет"), "привет");
        assert_eq!(format.format(1.5, false), "1.50");

        let quoted = NumberFormat::parse(r#""итого: "@"#);
        assert_eq!(quoted.format_text("42"), "итого: 42");
    }

    #[test]
    fn dates_in_the_1900_system() {
        assert_eq!(render(1.0, "yyyy-mm-dd"), "1900-01-01");
        assert_eq!(render(59.0, "yyyy-mm-dd"), "1900-02-28");
        // 29.02.1900 не существовало, но Excel его показывает.
        assert_eq!(render(60.0, "yyyy-mm-dd"), "1900-02-29");
        assert_eq!(render(61.0, "yyyy-mm-dd"), "1900-03-01");
        assert_eq!(render(45_000.0, "yyyy-mm-dd"), "2023-03-15");
    }

    #[test]
    fn date_separators_are_kept() {
        // Разделители внутри маски даты — часть записи, а не знаки чисел.
        assert_eq!(render(45_002.0, "dd.mm.yyyy"), "17.03.2023");
        assert_eq!(render(45_002.0, "d.m.yy"), "17.3.23");
        assert_eq!(render(45_002.0, "m/d/yyyy"), "3/17/2023");
        assert_eq!(render(45_002.0, "yyyy-mm-dd"), "2023-03-17");
        assert_eq!(render(45_002.5, "dd.mm.yyyy hh:mm"), "17.03.2023 12:00");
    }

    #[test]
    fn dates_in_the_1904_system() {
        assert_eq!(format(0.0, "yyyy-mm-dd", true), "1904-01-01");
        assert_eq!(format(1.0, "yyyy-mm-dd", true), "1904-01-02");
        assert_eq!(format(45_000.0, "yyyy-mm-dd", true), "2027-03-16");
    }

    #[test]
    fn weekday_matches_excel_including_the_phantom_day() {
        // 1900-01-01 Excel считает воскресеньем.
        assert_eq!(render(1.0, "dddd"), "Sunday");
        assert_eq!(render(7.0, "dddd"), "Saturday");
        // «Фантомный» день — среда по Excel.
        assert_eq!(render(60.0, "ddd"), "Wed");
        // 1900-03-01 — четверг, и это уже правда.
        assert_eq!(render(61.0, "ddd"), "Thu");
    }

    #[test]
    fn times_and_minutes() {
        assert_eq!(render(0.5, "hh:mm"), "12:00");
        assert_eq!(render(0.25, "hh:mm:ss"), "06:00:00");
        assert_eq!(render(0.5, "h:mm AM/PM"), "12:00 PM");
        assert_eq!(render(0.25, "h:mm AM/PM"), "6:00 AM");
        // `m` рядом с часами — минуты, а не месяц.
        assert_eq!(render(45000.5, "yyyy-mm-dd hh:mm"), "2023-03-15 12:00");
    }

    #[test]
    fn elapsed_time_does_not_wrap() {
        assert_eq!(render(1.5, "[h]:mm:ss"), "36:00:00");
        assert_eq!(render(0.5, "[m]"), "720");
        assert_eq!(render(0.5, "[s]"), "43200");
    }

    #[test]
    fn subsecond_precision_and_carry() {
        // 12:00:00.25
        assert_eq!(render(0.500_002_893_5, "hh:mm:ss.00"), "12:00:00.25");
        // Округление до секунды переносится в минуты.
        assert_eq!(render(0.499_999_9, "hh:mm:ss"), "12:00:00");
        assert_eq!(render(0.999_999_9, "hh:mm:ss"), "00:00:00");
    }

    #[test]
    fn month_and_day_names() {
        assert_eq!(render(45000.0, "d mmm yyyy"), "15 Mar 2023");
        assert_eq!(render(45000.0, "mmmm"), "March");
        assert_eq!(render(45000.0, "mmmmm"), "M");
    }

    #[test]
    fn negative_date_shows_hashes() {
        assert_eq!(render(-1.0, "yyyy-mm-dd"), "#####");
    }

    #[test]
    fn is_date_marks_date_formats() {
        assert!(NumberFormat::parse("yyyy-mm-dd").is_date());
        assert!(NumberFormat::parse("0.00;yyyy").is_date());
        assert!(!NumberFormat::parse("#,##0.00").is_date());
        assert!(!NumberFormat::parse("General").is_date());
    }

    #[test]
    fn skips_width_and_colors() {
        // `_(` — пропуск ширины символа, `[Red]` на текст не влияет.
        assert_eq!(render(5.0, "_(0.00_)"), " 5.00 ");
        assert_eq!(render(5.0, "[Red]0.00"), "5.00");
        assert_eq!(render(5.0, "0.00 \"кг\""), "5.00 кг");
        // Литералы между разрядами: телефонная маска.
        assert_eq!(render(1_234_567.0, r"000\-00\-00"), "123-45-67");
    }

    #[test]
    fn question_mark_aligns_decimals() {
        assert_eq!(render(1.5, "0.0?"), "1.5 ");
        assert_eq!(render(1.55, "0.0?"), "1.55");
    }

    #[test]
    fn zero_and_empty_codes() {
        assert_eq!(render(5.0, ""), "5");
        assert_eq!(render(0.0, "0;0;\"—\""), "—");
    }
}
