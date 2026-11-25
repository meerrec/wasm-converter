# ADR-0058: Шрифт эталона strict-viewer — Carlito в среде захвата

**Статус:** принят  
**Дата:** 25.11.2026  
**Контекст:** [`0056-capture-automation-com.md`](0056-capture-automation-com.md), [`0057-desktop-capture-envelope-mismatch.md`](0057-desktop-capture-envelope-mismatch.md), политика [`baseline-policy.md`](../strict-viewer/baseline-policy.md) §3 п.4, матрица [`certification-matrix.md`](../strict-viewer/certification-matrix.md) (`accept-environment`, §183)

## Контекст

Захват эталона для двух DOCX-фикстур (`docx.pdf`: `docx-tables-long-table`,
`docx-tables-long-table-header-repeat`) прошёл формально: drop импортирован,
`pnpm visual-oracle validate` говорит «corpus and 2 manifest(s) are valid».
Первый прогон оракула показал, что эталон **лжёт о своём окружении**.

| Что проверено | Факт |
|---|---|
| `pdffonts … /reference/export.pdf` | встроенный **`Calibri`** (`WinAnsi`, subset) |
| `manifest.environment.fonts` | `package: carlito-subset`, `version: 1.104` |
| SHA-256 в манифесте | совпадают с `crates/render/src/fonts/*.ttf` — с **нашим подрезанным** Carlito, а не с тем, что нарисовал Word |
| `w:rFonts` фикстуры | `w:ascii="Calibri"` — умолчание генератора (`scripts/docx-fixtures/kit.ts:303`) |
| Оракул | 4 × `visual-diff`: SSIM 0.876/0.881 и 0.956 |
| Геометрия | span первого текста `102–419`, строк с чернилами `1427` — совпадает **до пикселя** |
| Профиль строк | 741 из 1684 отличаются числом чернил; `pdftotext` идентичен |

Отпечаток — «та же вёрстка, другая гарнитура». В движке `Calibri` штатно
подменяется Carlito (`crates/render/src/font.rs`: `DEFAULT_FONT_ALIASES =
["Calibri"]`, метрический оригинал), поэтому вьюер нарисовал тем лицом, которым
обязан, а эталон — другим.

## Решение

**Эталонная среда — Carlito.** Она совпадает с тем, чем рисует вьюер, она
версионирована и с OFL-лицензией, и её можно поставить на машину захвата.

1. **Документы, попадающие в эталон, объявляют Carlito.** Умолчание
   `docDefaults` в фикстурах меняется с `Calibri` на `Carlito`; тогда
   документ **просит** ту гарнитуру, которой его потом нарисует вьюер.
2. **Среда захвата содержит полные Carlito 1.104.** В репозитории лежат
   подрезанные лица (465 кодпоинтов, 589 глифов) — они для рендера, не для
   установки. Машина захвата ставит полные файлы из пришпиленного коммита
   `googlefonts/carlito` (`3a810cab…`, SHA-256 `f6418f70…` и т. д. — те же,
   что проверяет `scripts/subset-fonts.sh`).
3. **Совпадение доказывается PDF-ом эталона, а не намерением.**
   `pdffonts` экспорта обязан показать Carlito. Если он показывает другую
   гарнитуру (в частности `Calibri`), кандидат в baseline отклоняется как
   дефект захвата.
4. **Подмена не скрывается, а проверяется.** Строка кадра по-прежнему несёт
   имя, которое назвал документ, — после смены умолчания это `Carlito`, и
   подмена `Calibri → Carlito` остаётся только для документов, которые
   реально просят Calibri (вне строгого корпуса).

Проверено на машине захвата:

```text
документ с w:ascii="Carlito" → Word 16.115 → save as PDF
pdffonts: AAAAAC+Carlito-Regular  TrueType  WinAnsi  emb yes  sub yes  uni yes
```

Тот же экспорт до установки полных Carlito давал `TimesNewRomanPSMT` (Word
молча подменял неизвестное семейство), поэтому требование п.2 — не
формальность.

## Последствия

- **Фикстуры перегенерируются.** Смена `docDefaults` меняет байты всех
  DOCX-фикстур: генератор детерминирован, но содержимое `styles.xml` другое.
  Оракулы `mammoth-oracle.json` и `docs/sprint-8/diff-report.md` обновляются
  вместе с ними, `diff:mammoth` обязан остаться на пороге.
- **Существующие эталоны отклоняются.** Две пары в
  `test-fixtures/strict-viewer/docx/docx-tables-long-table*/pdf` сняты с
  Calibri и подлежат пересъёмке; до неё это кандидаты, а не покрытие
  (`corpus.baselineId` остаётся `null`).
- **Допуск не трогается.** Глифовое расхождение при совпавшей геометрии
  по-прежнему `visual-diff`, а не «в пределах шума»: после пересъёмки оно
  должно исчезнуть; если останется — это настоящая регрессия рендера.
- **Другие форматы.** XLSX-фикстуры объявляют в теме `Cambria`
  (`xl/theme/theme1.xml`, `typeface="Cambria"`), а не Calibri; вьюер тоже
  подменяет её запасным лицом. Проверить `pdffonts` для XLSX нельзя: Excel на
  машине захвата отсутствует. Поэтому XLSX-эталоны не снимаются до появления
  Excel-среды, и пункт 1 к ним не применяется задним числом.
- **Процедура.** Пункт 2 дописывается в `docs/testing/strict-viewer-references.md`
  как обязательный шаг подготовки машины; проверка `pdffonts` входит в
  приёмку drop и в `.sh`/`.ps1`.

## Отклонённые варианты

- **Calibri как эталонная среда.** Совпадает с умолчанием фикстур без
  перегенерации, но требует настоящего Calibri у вьюера: это ломает
  `DEFAULT_FONT_ALIASES` и лицензионную модель (Calibri не OFL). Выбранный
  путь дешевле и не тянет проприетарный шрифт в продукт.
- **Поднять допуск под глифы.** Запрещено §5 и матрицей §183.
- **Записать `carlito-subset` и «поверить», что Word его использует.**
  `pdffonts` это опровергает; manifest существует для доказательства.
- **Подменить эталон кадром нашего вьюера.** Наш движок эталоном не является
  никогда (`baseline-policy.md` §3 п.5).

## Ссылки

- [`0056-capture-automation-com.md`](0056-capture-automation-com.md), [`0057-desktop-capture-envelope-mismatch.md`](0057-desktop-capture-envelope-mismatch.md).
- [`baseline-policy.md`](../strict-viewer/baseline-policy.md) — §3 п.4, §5.
- [`certification-matrix.md`](../strict-viewer/certification-matrix.md) — `accept-environment`, §183.
- `crates/render/src/font.rs` — `DEFAULT_FONT_ALIASES`.
- `crates/render/src/fonts/README.md`, `scripts/subset-fonts.sh` — версия 1.104, коммит, SHA-256.
