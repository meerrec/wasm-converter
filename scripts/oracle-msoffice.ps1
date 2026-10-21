#Requires -Version 5.1
<#
.SYNOPSIS
    Ручной SSIM-оракул Microsoft Office (Word/Excel) для нашего PDF-экспорта.

.DESCRIPTION
    Для каждой фикстуры .docx/.xlsx:
      1) Microsoft Word/Excel через COM экспортирует документ в PDF;
      2) наш release-пример doc-converter-pdf экспортирует тот же документ в PDF
         (docx_export для DOCX, mem_probe для XLSX; crates/pdf/examples);
      3) оба PDF растрируются pdftoppm в PNG при одном DPI;
      4) страницы сравниваются magick compare -metric SSIM;
      5) в -Out пишется report.md в стиле scripts/oracle-ssim.sh и
         scripts/oracle-docx-pages.sh.

    ImageMagick 7 печатает у compare -metric SSIM не SSIM, а distortion
    (проверено: одинаковые картинки дают 0, чёрное против белого — 0.49995).
    Скрипт калибрует режим на заведомо одинаковой картинке и переводит
    SSIM = 1 - 2 * distortion — так же, как это описано в шапке
    scripts/oracle-ssim.sh. Если калибровка не дала ни 0, ни 1, скрипт
    останавливается: молча писать в отчёт не то число хуже, чем упасть.

    Порог SSIM (mean и min) — 0.95, как у эталонных оракулов LibreOffice
    (docs/testing/docx-oracle.md, docs/sprint-12/oracle.md). Низкий SSIM —
    результат прогона, а не сбой скрипта: он попадает в отчёт и разбирается
    в docs/sprint-*/.

    Оракул ручной и в CI не входит: лицензии Microsoft Office и Windows-раннеры
    недоступны; действующий CI-оракул — LibreOffice. Скрипт не заменяет
    scripts/oracle-ssim.sh и scripts/oracle-docx-pages.sh, а дополняет их
    взглядом настоящего Word/Excel.

.PARAMETER Fixtures
    Каталоги фикстур (можно несколько). По умолчанию test-fixtures\docx и
    test-fixtures\xlsx. Относительные пути ищутся от текущего каталога, затем
    от корня репозитория.

.PARAMETER Out
    Каталог отчёта и артефактов. По умолчанию oracle-out-msoffice.

.PARAMETER Dpi
    DPI растеризации обоих PDF. По умолчанию 150 — как у эталонных скриптов.

.PARAMETER Threshold
    Порог SSIM для mean и min. По умолчанию 0.95.

.PARAMETER IncludeBroken
    Включить фикстуры из каталогов broken/ (по умолчанию пропускаются: наш
    разбор на них обязан падать, а Office может показать диалог восстановления).

.PARAMETER KeepPng
    Не удалять промежуточные PNG (pages/ и crop/). PDF и логи сохраняются
    всегда.

.PARAMETER SkipWord
    Не запускать Microsoft Word; .docx-фикстуры попадут в «пропущено».

.PARAMETER SkipExcel
    Не запускать Microsoft Excel; .xlsx-фикстуры попадут в «пропущено».

.NOTES
    Требования:
      * Windows 10/11 (COM-автоматизация и пути .exe);
      * установленные Microsoft Word и Excel (Word.Application/Excel.Application);
      * Rust 1.98.0 и cargo — release-сборка примеров doc-converter-pdf;
      * ImageMagick 7 с мультитулом magick (magick compare/identify);
      * Poppler (pdftoppm);
      * PowerShell 5.1+.

    Перед запуском закройте Word и Excel: скрипт создаёт COM-экземпляры и
    завершает их через Quit(), чтобы не закрыть пользовательские документы.

    Запуск:
      powershell -ExecutionPolicy Bypass -File scripts\oracle-msoffice.ps1 `
        -Fixtures test-fixtures\docx,test-fixtures\xlsx `
        -Out oracle-out-msoffice

    Коды выхода:
      0 — прогон состоялся (низкий SSIM и расхождение страниц — результат);
      1 — ни одной фикстуры не сравнено: сломан харнесс или экспорт;
      2 — нет предпосылок (не Windows, нет Office/COM, magick, pdftoppm,
          cargo, каталога фикстур) или неверное употребление.

    Коды отличаются от scripts/oracle-ssim.sh (там нет утилит — код 3):
    здесь все предпосылки, включая системные утилиты, сводятся к 2, чтобы
    вызывающая сторона различала «прогон не состоялся» и «низкий SSIM».
#>
[CmdletBinding()]
param(
    [Parameter(HelpMessage = 'Каталоги фикстур (.docx/.xlsx), можно несколько.')]
    [string[]]$Fixtures = @('test-fixtures\docx', 'test-fixtures\xlsx'),

    [Parameter(HelpMessage = 'Каталог отчёта и артефактов.')]
    [string]$Out = 'oracle-out-msoffice',

    [ValidateRange(72, 600)]
    [int]$Dpi = 150,

    [ValidateRange(0.0, 1.0)]
    [double]$Threshold = 0.95,

    [switch]$IncludeBroken,
    [switch]$KeepPng,
    [switch]$SkipWord,
    [switch]$SkipExcel
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# В PowerShell 7.3+ ненулевой код нативной команды может стать terminating
# error; compare возвращает 1 для различающихся картинок — это норма.
if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
    $PSNativeCommandUseErrorActionPreference = $false
}
# ImageMagick печатает метрику через точку только в C-локали.
$env:LC_ALL = 'C'
$env:LANG = 'C'
try {
    [Console]::OutputEncoding = [Text.Encoding]::UTF8
    $OutputEncoding = [Text.Encoding]::UTF8
} catch {
    # Консоль без UTF-8 — не повод падать: отчёт пишется в файл.
}

$script:MagickSsimIsDistortion = $false
$script:RepoRoot = $null
$script:WordVersion = 'не использовался'
$script:ExcelVersion = 'не использовался'

# --- общие помощники --------------------------------------------------------

function Fail {
    param([string]$Message)
    Write-Host "ОШИБКА: $Message" -ForegroundColor Red
    exit 2
}

function Add-Note {
    param($Row, [string]$Note)
    if ([string]::IsNullOrWhiteSpace($Note)) {
        return
    }
    if ([string]::IsNullOrWhiteSpace($Row.Note)) {
        $Row.Note = $Note
    } else {
        $Row.Note = "$($Row.Note); $Note"
    }
}

function Set-RowStatus {
    param($Row, [string]$Status, [int]$Rank, [string]$Note = '')
    # Статус не понижается: у фикстуры бывает и сбой сравнения, и низкий SSIM —
    # в отчёте важнее причина сбоя.
    if ($Rank -ge $Row.FailRank) {
        $Row.Status = $Status
        $Row.FailRank = $Rank
    }
    if (-not [string]::IsNullOrWhiteSpace($Note)) {
        Add-Note $Row $Note
    }
}

function Format-Ssim {
    param($Value)
    if ($null -eq $Value) {
        return '-'
    }
    if ($Value -is [string] -and $Value -eq '-') {
        return '-'
    }
    return ([double]$Value).ToString('0.0000', [Globalization.CultureInfo]::InvariantCulture)
}

function Format-Cell {
    param($Text)
    if ($null -eq $Text) {
        return ''
    }
    $flat = ([string]$Text) -replace "`r?`n", ' '
    return $flat.Replace('|', '\|')
}

function Format-PageRanges {
    param([int[]]$Pages)
    if ($null -eq $Pages -or $Pages.Count -eq 0) {
        return '-'
    }
    $sorted = @($Pages | Sort-Object -Unique)
    $parts = New-Object System.Collections.Generic.List[string]
    $start = $sorted[0]
    $prev = $sorted[0]
    for ($i = 1; $i -lt $sorted.Count; $i++) {
        $page = $sorted[$i]
        if ($page -eq $prev + 1) {
            $prev = $page
            continue
        }
        if ($start -eq $prev) {
            $parts.Add([string]$start)
        } else {
            $parts.Add("$start-$prev")
        }
        $start = $page
        $prev = $page
    }
    if ($start -eq $prev) {
        $parts.Add([string]$start)
    } else {
        $parts.Add("$start-$prev")
    }
    return ($parts -join ', ')
}

function Get-LastLine {
    param([string]$Text)
    if ([string]::IsNullOrWhiteSpace($Text)) {
        return ''
    }
    $lines = $Text -split "`r?`n"
    return $lines[$lines.Count - 1]
}

function Release-ComObject {
    param($ComObject)
    if ($null -ne $ComObject) {
        try {
            [void][Runtime.InteropServices.Marshal]::ReleaseComObject($ComObject)
        } catch {
            # Уже освобождён или процесс завершился — не повод падать.
        }
    }
}

function Require-Command {
    param([string]$Name, [string]$Hint)
    if (-not (Get-Command -Name $Name -ErrorAction SilentlyContinue)) {
        Fail "не найден '$Name'. $Hint"
    }
}

function Require-ComClass {
    param([string]$ProgId, [string]$Hint)
    $key = "Registry::HKEY_CLASSES_ROOT\$ProgId\CLSID"
    if (-not (Test-Path -LiteralPath $key)) {
        Fail "не зарегистрирован COM-класс '$ProgId'. $Hint"
    }
}

function Resolve-InputPath {
    param([string]$Path)
    if (Test-Path -LiteralPath $Path) {
        return (Resolve-Path -LiteralPath $Path).Path
    }
    $candidate = Join-Path $script:RepoRoot $Path
    if (Test-Path -LiteralPath $candidate) {
        return (Resolve-Path -LiteralPath $candidate).Path
    }
    return $null
}

function Build-Example {
    param([string]$Example)
    Push-Location $script:RepoRoot
    try {
        & cargo build --release -p doc-converter-pdf --example $Example
        if ($LASTEXITCODE -ne 0) {
            Fail "cargo build --release -p doc-converter-pdf --example $Example завершился с кодом $LASTEXITCODE."
        }
    } finally {
        Pop-Location
    }
}

function New-ComInstance {
    param([string]$ProgId, [string]$Hint)
    try {
        return New-Object -ComObject $ProgId
    } catch {
        Fail "не удалось запустить '$ProgId' через COM: $($_.Exception.Message). $Hint"
    }
}

# --- экспорт PDF ------------------------------------------------------------

function Export-WordPdf {
    param($Word, [string]$InputPath, [string]$OutputPath)
    $doc = $null
    try {
        # FileName, ConfirmConversions=false, ReadOnly=true.
        $doc = $Word.Documents.Open($InputPath, $false, $true)
        # wdExportFormatPDF = 17.
        $doc.ExportAsFixedFormat($OutputPath, 17)
    } finally {
        if ($null -ne $doc) {
            try { $doc.Close(0) } catch { }   # wdDoNotSaveChanges
            Release-ComObject $doc
        }
    }
}

function Export-ExcelPdf {
    param($Excel, [string]$InputPath, [string]$OutputPath)
    $workbook = $null
    try {
        # Filename, UpdateLinks=0, ReadOnly=true; экспорт книги целиком.
        $workbook = $Excel.Workbooks.Open($InputPath, 0, $true)
        # xlTypePDF = 0.
        $workbook.ExportAsFixedFormat(0, $OutputPath)
    } finally {
        if ($null -ne $workbook) {
            try { $workbook.Close($false) } catch { }
            Release-ComObject $workbook
        }
    }
}

function Export-OurPdf {
    param([string]$Exporter, [string]$InputPath, [string]$OutputPath, [string]$LogPath)
    $output = & $Exporter $InputPath --out $OutputPath 2>&1
    $code = $LASTEXITCODE
    $text = (($output | Out-String).Trim())
    if (-not [string]::IsNullOrWhiteSpace($text)) {
        Set-Content -LiteralPath $LogPath -Value $text -Encoding UTF8
    }
    if ($code -ne 0) {
        $reason = Get-LastLine $text
        throw "$(Split-Path -Leaf $Exporter): код $code$(if ($reason) { " — $reason" })"
    }
    if (-not (Test-Path -LiteralPath $OutputPath) -or (Get-Item -LiteralPath $OutputPath).Length -eq 0) {
        throw "$(Split-Path -Leaf $Exporter): PDF не создан"
    }
}

# --- растеризация и SSIM ----------------------------------------------------

function Get-PageNumber {
    param([string]$Path)
    if ($Path -match '-(\d+)\.png$') {
        return [int]$Matches[1]
    }
    return 0
}

function Get-PageImages {
    param([string]$Prefix)
    $dir = Split-Path -Parent $Prefix
    $leaf = Split-Path -Leaf $Prefix
    if (-not (Test-Path -LiteralPath $dir)) {
        return @()
    }
    $files = @(Get-ChildItem -LiteralPath $dir -File -Filter "$leaf-*.png" -ErrorAction SilentlyContinue)
    return @($files | Sort-Object { Get-PageNumber -Path $_.FullName })
}

function Invoke-Rasterize {
    param([string]$Pdf, [string]$Prefix, [string]$LogPath)
    $dir = Split-Path -Parent $Prefix
    $leaf = Split-Path -Leaf $Prefix
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Get-ChildItem -LiteralPath $dir -File -Filter "$leaf-*.png" -ErrorAction SilentlyContinue |
        Remove-Item -Force -ErrorAction SilentlyContinue
    $output = & pdftoppm -png -gray -r $Dpi $Pdf $Prefix 2>&1
    $code = $LASTEXITCODE
    $text = (($output | Out-String).Trim())
    if (-not [string]::IsNullOrWhiteSpace($text)) {
        Set-Content -LiteralPath $LogPath -Value $text -Encoding UTF8
    }
    if ($code -ne 0) {
        throw "pdftoppm: код $code$(if ($text) { " — $(Get-LastLine $text)" })"
    }
    $pages = @(Get-PageImages $Prefix)
    if ($pages.Count -eq 0) {
        throw 'pdftoppm не создал ни одной страницы'
    }
    return $pages
}

function Get-ImageSize {
    param([string]$Path)
    $raw = & magick identify -quiet -format '%w %h' $Path 2>$null
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        return $null
    }
    $text = (($raw | Out-String).Trim())
    if ($text -match '^(\d+)\s+(\d+)$') {
        return [pscustomobject]@{ Width = [int]$Matches[1]; Height = [int]$Matches[2] }
    }
    return $null
}

function Get-MagickRawSsim {
    param([string]$A, [string]$B)
    # compare пишет метрику в stderr; код 1 — картинки различаются, это норма.
    $raw = & magick compare -metric SSIM $A $B null: 2>&1
    $code = $LASTEXITCODE
    $text = (($raw | Out-String).Trim())
    if ($code -gt 1) {
        throw "magick compare: код $code — $text"
    }
    $value = $null
    # IM7 печатает «<Q16-значение> (<нормированное>)»; для SSIM нормированное —
    # distortion. Берём именно скобки, а не первое число.
    if ($text -match '\(([-+]?\d+(?:[.,]\d+)?(?:[eE][-+]?\d+)?)\)') {
        $value = [double]::Parse(($Matches[1] -replace ',', '.'), [Globalization.CultureInfo]::InvariantCulture)
    } elseif ($text -match '([-+]?\d+(?:[.,]\d+)?(?:[eE][-+]?\d+)?)\s*$') {
        $value = [double]::Parse(($Matches[1] -replace ',', '.'), [Globalization.CultureInfo]::InvariantCulture)
    } else {
        throw "magick compare не напечатал метрику: $text"
    }
    # На случай версии без скобок: Q16-значение нормируется в 0..1.
    if ($value -gt 1.0) {
        $value = $value / 65535.0
    }
    return $value
}

function ConvertTo-Ssim {
    param([double]$Raw)
    if ($script:MagickSsimIsDistortion) {
        $value = 1.0 - 2.0 * $Raw
    } else {
        $value = $Raw
    }
    if ($value -lt -1.0) { $value = -1.0 }
    if ($value -gt 1.0) { $value = 1.0 }
    return $value
}

function Initialize-MagickSsimMode {
    param([string]$Dir)
    $calibration = Join-Path $Dir 'calibration.png'
    & magick -size 8x8 xc:white $calibration 2>$null
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $calibration)) {
        Fail 'ImageMagick не создал калибровочное изображение (magick -size 8x8 xc:white).'
    }
    try {
        $raw = Get-MagickRawSsim -A $calibration -B $calibration
    } catch {
        Fail "калибровка ImageMagick не удалась: $($_.Exception.Message)"
    }
    if ([Math]::Abs($raw) -lt 0.01) {
        # IM7: compare -metric SSIM печатает distortion, SSIM = 1 - 2*distortion.
        $script:MagickSsimIsDistortion = $true
        return
    }
    if ([Math]::Abs($raw - 1.0) -lt 0.01) {
        $script:MagickSsimIsDistortion = $false
        return
    }
    Fail "magick compare -metric SSIM на одинаковых изображениях вернул $raw; ожидалось 0 (distortion) или 1 (SSIM)."
}

function Compare-Pages {
    param([string]$A, [string]$B, [string]$CropDir, [string]$Stem, [int]$Page)
    $sizeA = Get-ImageSize $A
    $sizeB = Get-ImageSize $B
    if ($null -eq $sizeA -or $null -eq $sizeB) {
        return [pscustomobject]@{ Error = 'не определил размеры страниц' }
    }
    $sizes = "$($sizeA.Width)x$($sizeA.Height) и $($sizeB.Width)x$($sizeB.Height)"
    $dw = [Math]::Abs($sizeA.Width - $sizeB.Width)
    $dh = [Math]::Abs($sizeA.Height - $sizeB.Height)
    if ($dw -gt 2 -or $dh -gt 2) {
        # Больше 2 px — разные бумага или масштаб; SSIM по ним ничего не значит.
        return [pscustomobject]@{ PaperMismatch = $true; Sizes = $sizes }
    }
    $cropped = $false
    if ($dw -ne 0 -or $dh -ne 0) {
        # До 2 px — округление сетки растеризации (A4 у Word и у нас может
        # выйти 1241x1754 против 1240x1755). Обрезка, в отличие от ресайза, не
        # размывает края и не занижает метрику.
        $width = [Math]::Min($sizeA.Width, $sizeB.Width)
        $height = [Math]::Min($sizeA.Height, $sizeB.Height)
        $cropA = Join-Path $CropDir "$Stem-$Page-office.png"
        $cropB = Join-Path $CropDir "$Stem-$Page-our.png"
        & magick $A -crop "$($width)x$($height)+0+0" +repage $cropA 2>$null
        if ($LASTEXITCODE -ne 0) {
            return [pscustomobject]@{ Error = 'не обрезал страницу Office до общего размера' }
        }
        & magick $B -crop "$($width)x$($height)+0+0" +repage $cropB 2>$null
        if ($LASTEXITCODE -ne 0) {
            return [pscustomobject]@{ Error = 'не обрезал нашу страницу до общего размера' }
        }
        $A = $cropA
        $B = $cropB
        $cropped = $true
    }
    try {
        $raw = Get-MagickRawSsim -A $A -B $B
    } catch {
        return [pscustomobject]@{ Error = $_.Exception.Message }
    }
    return [pscustomobject]@{ Ssim = (ConvertTo-Ssim -Raw $raw); Cropped = $cropped; Sizes = $sizes }
}

function Get-XlsxSheetCount {
    param([string]$Path)
    $zip = $null
    $reader = $null
    try {
        $zip = [System.IO.Compression.ZipFile]::OpenRead($Path)
        $entry = $zip.GetEntry('xl/workbook.xml')
        if ($null -eq $entry) {
            return 0
        }
        $reader = New-Object System.IO.StreamReader($entry.Open())
        $xml = $reader.ReadToEnd()
        return ([regex]::Matches($xml, '<sheet\b')).Count
    } catch {
        return -1
    } finally {
        if ($null -ne $reader) { $reader.Dispose() }
        if ($null -ne $zip) { $zip.Dispose() }
    }
}

# --- предпосылки ------------------------------------------------------------

if ($env:OS -ne 'Windows_NT') {
    Fail 'скрипт рассчитан на Windows: Microsoft Word/Excel COM и пути .exe. Запускайте на Windows с установленным Office.'
}
if ($Threshold -le 0.0) {
    Fail "порог SSIM должен быть больше 0 (получено $Threshold)."
}

$script:RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
if (-not (Test-Path -LiteralPath (Join-Path $script:RepoRoot 'Cargo.toml'))) {
    Fail "не найден Cargo.toml в корне репозитория: $($script:RepoRoot)"
}

try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop
} catch {
    Fail "не удалось загрузить System.IO.Compression.FileSystem (нужен .NET 4.5+): $($_.Exception.Message)"
}

Require-Command 'cargo' 'Установите Rust 1.98.0 (rustup) и добавьте cargo в PATH.'
Require-Command 'magick' 'Установите ImageMagick 7 (мультитул magick) и добавьте его в PATH.'
Require-Command 'pdftoppm' 'Установите Poppler (pdftoppm) и добавьте его в PATH.'

$magickVersionLine = ((& magick -version 2>&1 | Select-Object -First 1) | Out-String).Trim()
if ($magickVersionLine -notmatch 'ImageMagick\s+7\.') {
    Fail "нужен ImageMagick 7: compare -metric SSIM есть только там (найдено: $magickVersionLine)."
}
$popplerVersionLine = ((& pdftoppm -v 2>&1 | Select-Object -First 1) | Out-String).Trim()
$cargoVersionLine = ((& cargo --version 2>&1 | Select-Object -First 1) | Out-String).Trim()

# --- отбор фикстур ----------------------------------------------------------

$fixtureItems = New-Object System.Collections.Generic.List[object]
foreach ($root in $Fixtures) {
    $resolved = Resolve-InputPath $root
    if ($null -eq $resolved) {
        Fail "каталог фикстур не найден: $root"
    }
    if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
        Fail "не каталог фикстур: $root"
    }
    $rootLength = $resolved.TrimEnd('\', '/').Length
    Get-ChildItem -LiteralPath $resolved -Recurse -File | ForEach-Object {
        $extension = $_.Extension.ToLowerInvariant()
        if ($extension -ne '.docx' -and $extension -ne '.xlsx') {
            return
        }
        if (-not $IncludeBroken -and $_.FullName -match '[\\/]broken[\\/]') {
            return
        }
        $relative = $_.FullName.Substring($rootLength).TrimStart('\', '/')
        $stem = ($relative -replace '\.(docx|xlsx)$', '') -replace '[\\/]', '_'
        $fixtureItems.Add([pscustomobject]@{
            File = $_
            Rel = $relative
            Stem = $stem
            Kind = $(if ($extension -eq '.docx') { 'docx' } else { 'xlsx' })
            Name = $relative
        })
    }
}

# Пересекающиеся каталоги не должны дать одну фикстуру дважды.
$uniqueItems = New-Object System.Collections.Generic.List[object]
$seenPaths = @{}
foreach ($item in $fixtureItems) {
    if (-not $seenPaths.ContainsKey($item.File.FullName)) {
        $seenPaths[$item.File.FullName] = $true
        $uniqueItems.Add($item)
    }
}
$fixtureItems = $uniqueItems

if ($fixtureItems.Count -eq 0) {
    Fail "в указанных каталогах нет фикстур .docx/.xlsx (Fixtures: $($Fixtures -join ', '))."
}

$docxItems = @($fixtureItems | Where-Object { $_.Kind -eq 'docx' })
$xlsxItems = @($fixtureItems | Where-Object { $_.Kind -eq 'xlsx' })
$needWord = ($docxItems.Count -gt 0) -and (-not $SkipWord)
$needExcel = ($xlsxItems.Count -gt 0) -and (-not $SkipExcel)
if (-not $needWord -and -not $needExcel) {
    Fail 'нечего проверять: все найденные форматы отключены через -SkipWord/-SkipExcel.'
}

if ($needWord) {
    Require-ComClass 'Word.Application' 'Установите Microsoft Word (нужна COM-автоматизация).'
}
if ($needExcel) {
    Require-ComClass 'Excel.Application' 'Установите Microsoft Excel (нужна COM-автоматизация).'
}

# --- сборка наших экспортёров ----------------------------------------------

$docxExport = $null
$memProbe = $null
if ($needWord) {
    Build-Example 'docx_export'
    $docxExport = Join-Path $script:RepoRoot 'target\release\examples\docx_export.exe'
    if (-not (Test-Path -LiteralPath $docxExport)) {
        Fail "cargo не создал $docxExport"
    }
}
if ($needExcel) {
    Build-Example 'mem_probe'
    $memProbe = Join-Path $script:RepoRoot 'target\release\examples\mem_probe.exe'
    if (-not (Test-Path -LiteralPath $memProbe)) {
        Fail "cargo не создал $memProbe"
    }
}

# --- каталог отчёта ---------------------------------------------------------

if (-not [IO.Path]::IsPathRooted($Out)) {
    $Out = Join-Path (Get-Location).Path $Out
}
$Out = [IO.Path]::GetFullPath($Out)
$refDir = Join-Path $Out 'ref'
$ourDir = Join-Path $Out 'our'
$pagesOfficeDir = Join-Path $Out 'pages\office'
$pagesOurDir = Join-Path $Out 'pages\our'
$cropDir = Join-Path $Out 'crop'
$logDir = Join-Path $Out 'log'
foreach ($dir in @($Out, $refDir, $ourDir, $pagesOfficeDir, $pagesOurDir, $cropDir, $logDir)) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
}

Initialize-MagickSsimMode -Dir $Out

# --- прогон -----------------------------------------------------------------

$results = New-Object System.Collections.Generic.List[object]
$allPages = New-Object System.Collections.Generic.List[object]
$compared = 0
$skipped = 0
$failed = 0
$partialErrors = 0
$pageMismatches = 0
$belowThreshold = 0

$word = $null
$excel = $null
try {
    if ($needWord) {
        $word = New-ComInstance 'Word.Application' 'Установите Microsoft Word и повторите запуск.'
        try {
            if ($word.Documents.Count -gt 0) {
                Fail 'Word уже запущен с открытыми документами; закройте Word перед запуском оракула.'
            }
        } catch {
            # Свойство может быть недоступно в момент старта — не повод падать.
        }
        try { $word.Visible = $false } catch { }
        try { $word.DisplayAlerts = 0 } catch { }
        try { $word.AutomationSecurity = 3 } catch { }   # force-disable macros
        try { $script:WordVersion = [string]$word.Version } catch { }
    }
    if ($needExcel) {
        $excel = New-ComInstance 'Excel.Application' 'Установите Microsoft Excel и повторите запуск.'
        try {
            if ($excel.Workbooks.Count -gt 0) {
                Fail 'Excel уже запущен с открытыми книгами; закройте Excel перед запуском оракула.'
            }
        } catch {
            # См. комментарий выше.
        }
        try { $excel.Visible = $false } catch { }
        try { $excel.DisplayAlerts = $false } catch { }
        try { $excel.AutomationSecurity = 3 } catch { }
        try { $script:ExcelVersion = [string]$excel.Version } catch { }
    }

    foreach ($item in $fixtureItems) {
        $row = [pscustomobject]@{
            Name = $item.Name
            Kind = $item.Kind
            Status = 'сравнено'
            FailRank = 0
            OfficePages = '-'
            OurPages = '-'
            Delta = '-'
            First = $null
            Min = $null
            Mean = $null
            Note = ''
        }
        $results.Add($row)

        if ($item.Kind -eq 'docx' -and $SkipWord) {
            $row.Status = 'пропущено'
            $row.Note = 'пропущено: -SkipWord'
            $skipped++
            continue
        }
        if ($item.Kind -eq 'xlsx' -and $SkipExcel) {
            $row.Status = 'пропущено'
            $row.Note = 'пропущено: -SkipExcel'
            $skipped++
            continue
        }
        if ($item.Kind -eq 'xlsx') {
            $sheetCount = Get-XlsxSheetCount $item.File.FullName
            if ($sheetCount -gt 1) {
                # Наш экспорт рисует лист 0, Office печатает всю книгу: на второй
                # странице эталона оказался бы другой лист. Та же причина, что в
                # scripts/oracle-ssim.sh.
                $row.Status = 'пропущено'
                $row.Note = "многолистовая книга ($sheetCount листов): наш экспорт рисует лист 0, Office печатает всю книгу"
                $skipped++
                continue
            }
        }

        $officePdf = Join-Path $refDir "$($item.Stem).pdf"
        $ourPdf = Join-Path $ourDir "$($item.Stem).pdf"
        Remove-Item -LiteralPath $officePdf -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $ourPdf -Force -ErrorAction SilentlyContinue

        try {
            if ($item.Kind -eq 'docx') {
                Export-WordPdf -Word $word -InputPath $item.File.FullName -OutputPath $officePdf
            } else {
                Export-ExcelPdf -Excel $excel -InputPath $item.File.FullName -OutputPath $officePdf
            }
            if (-not (Test-Path -LiteralPath $officePdf) -or (Get-Item -LiteralPath $officePdf).Length -eq 0) {
                throw 'Office не создал PDF (файл отсутствует или пуст)'
            }
        } catch {
            Set-RowStatus $row 'Office не открыл' 3 $_.Exception.Message
            $failed++
            continue
        }

        try {
            if ($item.Kind -eq 'docx') {
                Export-OurPdf -Exporter $docxExport -InputPath $item.File.FullName -OutputPath $ourPdf `
                    -LogPath (Join-Path $logDir "$($item.Stem).our.log")
            } else {
                Export-OurPdf -Exporter $memProbe -InputPath $item.File.FullName -OutputPath $ourPdf `
                    -LogPath (Join-Path $logDir "$($item.Stem).our.log")
            }
        } catch {
            Set-RowStatus $row 'сбой экспорта' 2 $_.Exception.Message
            $failed++
            continue
        }

        try {
            $officeImages = @(Invoke-Rasterize -Pdf $officePdf `
                -Prefix (Join-Path $pagesOfficeDir $item.Stem) `
                -LogPath (Join-Path $logDir "$($item.Stem).office.ppm.log"))
        } catch {
            Set-RowStatus $row 'сбой сравнения' 3 "Office PDF: $($_.Exception.Message)"
            $failed++
            continue
        }
        try {
            $ourImages = @(Invoke-Rasterize -Pdf $ourPdf `
                -Prefix (Join-Path $pagesOurDir $item.Stem) `
                -LogPath (Join-Path $logDir "$($item.Stem).our.ppm.log"))
        } catch {
            Set-RowStatus $row 'сбой экспорта' 2 "наш PDF: $($_.Exception.Message)"
            $failed++
            continue
        }

        $row.OfficePages = $officeImages.Count
        $row.OurPages = $ourImages.Count
        $row.Delta = ([int]$ourImages.Count) - ([int]$officeImages.Count)
        $shared = [Math]::Min($officeImages.Count, $ourImages.Count)
        # DOCX: критерий DoD «±1 страница»; XLSX: страницы должны совпадать,
        # иначе сравнивать нечего (как в scripts/oracle-ssim.sh).
        $tolerance = $(if ($item.Kind -eq 'docx') { 1 } else { 0 })
        $outside = [Math]::Abs([int]$row.Delta) -gt $tolerance
        if ($outside) {
            $pageMismatches++
            Set-RowStatus $row 'расхождение страниц' 1 `
                "страниц у Office $($officeImages.Count), у нас $($ourImages.Count) (Δ $($row.Delta), допуск ±$tolerance)"
        } elseif ([int]$row.Delta -ne 0) {
            Add-Note $row "страниц у Office $($officeImages.Count), у нас $($ourImages.Count) — в допуске ±$tolerance"
        }

        $pageResults = New-Object System.Collections.Generic.List[object]
        $pageErrors = New-Object System.Collections.Generic.List[string]
        $cropPages = New-Object System.Collections.Generic.List[int]
        $paperPages = New-Object System.Collections.Generic.List[int]
        $cropSizes = $null
        $paperSizes = $null

        for ($i = 0; $i -lt $shared; $i++) {
            $page = $i + 1
            $result = Compare-Pages -A $officeImages[$i].FullName -B $ourImages[$i].FullName `
                -CropDir $cropDir -Stem $item.Stem -Page $page
            if ($result.PaperMismatch) {
                $paperPages.Add($page)
                if ($null -eq $paperSizes) { $paperSizes = $result.Sizes }
                continue
            }
            if ($result.Error) {
                $pageErrors.Add("страница ${page}: $($result.Error)")
                continue
            }
            $ssim = [double]$result.Ssim
            $pageResults.Add([pscustomobject]@{ Page = $page; Ssim = $ssim })
            $allPages.Add([pscustomobject]@{ Fixture = $row.Name; Page = $page; Ssim = $ssim })
            if ($result.Cropped) {
                $cropPages.Add($page)
                if ($null -eq $cropSizes) { $cropSizes = $result.Sizes }
            }
        }

        if ($paperPages.Count -gt 0) {
            Add-Note $row "страницы $(Format-PageRanges $paperPages.ToArray()): размеры $paperSizes — разные бумага или масштаб, SSIM не считан"
            Set-RowStatus $row 'сбой сравнения' 2 ''
        }
        if ($cropPages.Count -gt 0) {
            Add-Note $row "страницы $(Format-PageRanges $cropPages.ToArray()): размеры $cropSizes — расхождение ≤2 px, сравнено по фрагменту"
        }
        if ($pageErrors.Count -gt 0) {
            foreach ($pageError in $pageErrors) {
                Add-Note $row $pageError
            }
            Set-RowStatus $row 'сбой сравнения' 2 ''
        }

        if ($pageResults.Count -eq 0) {
            Add-Note $row 'ни одной страницы не сравнено'
            if ($row.FailRank -lt 2) {
                Set-RowStatus $row 'сбой сравнения' 2 ''
            }
            $failed++
            continue
        }

        $stats = $pageResults | Measure-Object -Property Ssim -Minimum -Average
        $row.Min = [double]$stats.Minimum
        $row.Mean = [double]$stats.Average
        $firstPage = $pageResults | Where-Object { $_.Page -eq 1 } | Select-Object -First 1
        if ($null -ne $firstPage) {
            $row.First = [double]$firstPage.Ssim
        }

        $low = (($row.Mean -lt $Threshold) -or ($row.Min -lt $Threshold))
        if ($low) {
            $belowThreshold++
            Add-Note $row ("mean {0}, min {1} ниже порога {2}" -f `
                (Format-Ssim $row.Mean), (Format-Ssim $row.Min), (Format-Ssim $Threshold))
            if (-not $outside -and $row.FailRank -lt 1) {
                $row.Status = 'ниже порога'
                $row.FailRank = 1
            }
        }

        if ($pageErrors.Count -gt 0 -or $paperPages.Count -gt 0) {
            $partialErrors++
        }
        $compared++
    }
} finally {
    if ($null -ne $word) {
        try { $word.Quit() } catch { }
        Release-ComObject $word
    }
    if ($null -ne $excel) {
        try { $excel.Quit() } catch { }
        Release-ComObject $excel
    }
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
}

# --- отчёт ------------------------------------------------------------------

$reportPath = Join-Path $Out 'report.md'
$comparedRows = @($results | Where-Object { $null -ne $_.Mean })
$comparedCount = $comparedRows.Count
$allPageValues = @($allPages | ForEach-Object { $_.Ssim })
$allMin = $null
$allMean = $null
$goodPages = 0
if ($allPageValues.Count -gt 0) {
    $allStats = $allPageValues | Measure-Object -Minimum -Average
    $allMin = [double]$allStats.Minimum
    $allMean = [double]$allStats.Average
    $goodPages = @($allPages | Where-Object { $_.Ssim -ge $Threshold }).Count
}
$goodFixtures = @($comparedRows | Where-Object {
    $_.Mean -ge $Threshold -and $_.Min -ge $Threshold
}).Count

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('# MS Office SSIM-оракул')
$lines.Add('')
$lines.Add("- Дата: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')")
$lines.Add("- Машина: $env:COMPUTERNAME")
$lines.Add("- ОС: $([Environment]::OSVersion.VersionString)")
$lines.Add("- Microsoft Word: $($script:WordVersion)")
$lines.Add("- Microsoft Excel: $($script:ExcelVersion)")
$lines.Add("- Rust: $cargoVersionLine")
$lines.Add("- ImageMagick: $magickVersionLine")
$lines.Add("- Poppler: $popplerVersionLine")
$lines.Add("- DPI: $Dpi, растеризация ``pdftoppm -png -gray -r $Dpi``")
$lines.Add("- Порог: SSIM ≥ $(Format-Ssim $Threshold) (mean и min фикстуры)")
$lines.Add('- Наш экспорт: `docx_export` и `mem_probe` (release, `crates/pdf/examples`)')
$lines.Add('- Сравнение: `magick compare -metric SSIM`; distortion калибруется в SSIM = 1 − 2·distortion')
if ($IncludeBroken) {
    $lines.Add('- Фикстуры `broken/` включены через `-IncludeBroken`')
}
$lines.Add('')
$lines.Add('Методика: Office экспортирует документ в PDF через COM, наш пример — в свой PDF; оба PDF растрируются одним DPI и сравниваются постранично. Числа — материал для разбора, а не CI-гейт: оракул ручной, Microsoft Office в CI не входит.')
$lines.Add('')
$lines.Add('| Фикстура | Формат | Страниц Office | Страниц ours | Δ | SSIM 1-й | min | mean | Статус | Примечание |')
$lines.Add('| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |')
foreach ($row in ($results | Sort-Object Name)) {
    $delta = '-'
    if ($row.Delta -ne '-') {
        $delta = '{0:+0;-0;0}' -f [int]$row.Delta
    }
    $lines.Add("| $(Format-Cell $row.Name) | $($row.Kind) | $($row.OfficePages) | $($row.OurPages) | $delta | $(Format-Ssim $row.First) | $(Format-Ssim $row.Min) | $(Format-Ssim $row.Mean) | $($row.Status) | $(Format-Cell $row.Note) |")
}
$lines.Add('')
$lines.Add('## Итоги')
$lines.Add('')
$lines.Add("Сравнено: $comparedCount, пропущено: $skipped, с ошибкой: $failed (не сравнено), частичных ошибок: $partialErrors.")
$lines.Add("SSIM страниц: min $(Format-Ssim $allMin), mean $(Format-Ssim $allMean), ≥ $(Format-Ssim $Threshold) у $goodPages из $($allPages.Count).")
$lines.Add("Фикстуры: mean и min ≥ порога у $goodFixtures из $comparedCount; ниже порога: $belowThreshold; расхождение страниц: $pageMismatches.")
$lines.Add('')
$lines.Add('## Худшие 10 страниц')
$lines.Add('')
if ($allPages.Count -eq 0) {
    $lines.Add('Нет сравнённых страниц.')
} else {
    $lines.Add('| Фикстура | Страница | SSIM |')
    $lines.Add('| --- | ---: | ---: |')
    foreach ($page in ($allPages | Sort-Object Ssim | Select-Object -First 10)) {
        $lines.Add("| $(Format-Cell $page.Fixture) | $($page.Page) | $(Format-Ssim $page.Ssim) |")
    }
}
$lines.Add('')
$lines.Add('## Ошибки')
$lines.Add('')
$errorRows = @($results | Where-Object { $_.Status -in @('Office не открыл', 'сбой экспорта', 'сбой сравнения') })
if ($errorRows.Count -eq 0) {
    $lines.Add('Нет.')
} else {
    foreach ($row in $errorRows) {
        $lines.Add("- **$($row.Name)** — $($row.Status): $(Format-Cell $row.Note)")
    }
}
$lines.Add('')
$lines.Add('## Пропущенные')
$lines.Add('')
$skipRows = @($results | Where-Object { $_.Status -eq 'пропущено' })
if ($skipRows.Count -eq 0) {
    $lines.Add('Нет.')
} else {
    foreach ($row in $skipRows) {
        $lines.Add("- **$($row.Name)** — $(Format-Cell $row.Note)")
    }
}
$lines.Add('')
$lines.Add('## Замечания')
$lines.Add('')
$lines.Add('- Растеризация `-gray`: метрика про геометрию, шрифт и пагинацию; цвет и заливки ею не проверяются — так же, как в `scripts/oracle-ssim.sh`.')
$lines.Add('- ImageMagick 7 печатает у `compare -metric SSIM` нормированный distortion; скрипт калибрует режим на одинаковой картинке и переводит `SSIM = 1 − 2·distortion`. Если калибровка не дала 0 или 1, прогон останавливается с кодом 2.')
$lines.Add('- Эталон — Microsoft Office, а не LibreOffice: числа не сравнимы напрямую с `scripts/oracle-ssim.sh` и `scripts/oracle-docx-pages.sh` (другой рендерер и другой SSIM-бэкенд). Сравнивайте прогоны MS Office между собой.')
$lines.Add('- DOCX: допуск числа страниц ±1 (критерий DoD); XLSX: страницы должны совпадать, при расхождении сравниваются первые min(N) страниц.')
$lines.Add('- Многолистовые XLSX пропускаются: наш экспорт рисует лист 0, Office печатает всю книгу; постраничное сравнение мерило бы разные листы.')
$lines.Add('- Фикстуры `broken/` пропускаются по умолчанию: наш разбор на них обязан падать. Флаг `-IncludeBroken` возвращает их ради разбора сбоев.')
$lines.Add('- PDF эталона и наши — в `ref/` и `our/`, логи — в `log/`; PNG удаляются, если не задан `-KeepPng`.')
$lines.Add('')
$lines.Add('## Артефакты')
$lines.Add('')
$lines.Add("- `ref/` — PDF Microsoft Office; `our/` — наш PDF; `log/` — stdout/stderr примеров и pdftoppm.")
if ($KeepPng) {
    $lines.Add('- `pages/` и `crop/` — растрированные страницы и обрезанные фрагменты (`-KeepPng`).')
}
$lines.Add('')
$lines.Add('Расхождения разбираются в отчётах `docs/sprint-*/` (пример формата — `docs/sprint-12/oracle.md`), а не в CI.')

Set-Content -LiteralPath $reportPath -Value $lines.ToArray() -Encoding UTF8

if (-not $KeepPng) {
    Remove-Item -LiteralPath (Join-Path $Out 'pages') -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath (Join-Path $Out 'crop') -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath (Join-Path $Out 'calibration.png') -Force -ErrorAction SilentlyContinue
}

Write-Host ''
Write-Host ($lines -join [Environment]::NewLine)
Write-Host ''
Write-Host "Отчёт: $reportPath"

if ($comparedCount -eq 0 -and $failed -gt 0) {
    Write-Host "ОШИБКА: ни одной фикстуры не сравнено; смотрите $reportPath" -ForegroundColor Red
    exit 1
}
if ($comparedCount -eq 0) {
    Write-Host "ПРЕДУПРЕЖДЕНИЕ: нечего сравнивать — все фикстуры пропущены; смотрите $reportPath" -ForegroundColor Yellow
}
exit 0
