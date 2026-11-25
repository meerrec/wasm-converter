#Requires -Version 5.1
<#
.SYNOPSIS
    Снимает эталоны strict-viewer из документов корпуса локальным Microsoft Word/Excel через COM.

.DESCRIPTION
    Конвейер для каждой фикстуры и каждого режима из test-fixtures/strict-viewer/corpus.json:
      1) Microsoft Word/Excel через COM экспортирует документ в PDF (ExportAsFixedFormat);
      2) PDF растрируется `pdftoppm -png -r 144` (без -gray: эталону нужен цвет);
      3) PNG проверяются: сигнатура и IHDR, bit depth 8, colorType 2 (RGB8) или 6 (RGBA8),
         размер не больше 4 МиБ;
      4) собирается drop <Out>/drop/<format>/<fixture-id>/<mode>/ с capture.json
         и reference/export.pdf;
      5) drop импортируется в manifest через `pnpm visual-oracle reference import`;
      6) в <Out>/report.md пишется таблица по фикстурам и режимам.

    ОГОВОРКА О ПОЛИТИКЕ. Скрипт снимает эталон локальным Microsoft Word/Excel через
    COM. Это разрешено решением `accept-capture-automation` (docs/adr/0056-capture-automation-com.md,
    docs/strict-viewer/baseline-policy.md §1 и §3 п.5): с v1.1.0 политики эталон снимается
    автоматически либо вручную в Office Online — способ захвата на выбор. Принятие
    baseline не изменилось: reference от скрипта — кандидат, а baseline принимается
    отдельным baseline-only PR с двумя approvals и независимым повторным захватом (§5).

    Режимы docx.print-layout и xlsx.normal снимаются с экрана веб-версии и из PDF-экспорта
    не получаются. По умолчанию они пропускаются (Skip) с явной причиной в отчёте; флаг
    -IncludeWebModes лишь меняет формулировку причины на «требует ручного захвата,
    не реализовано». Эквивалент не выдумывается.

    Скрипт не обращается к Office Online: ни URL, ни сетевых вызовов, ни учётных данных.
    Эталон снимает локальный Word/Excel, установленный на машине.

    КОНВЕРТ MANIFEST. Захват локальным Office записывается честно: reference.provider =
    office-desktop, reference.application = word-desktop (docx) либо excel-desktop (xlsx),
    reference.applicationVersion = $word.Version / $excel.Version, reference.captureMethod =
    automated. Ручной захват в Office Online остаётся возможным: он даёт office-online,
    *-online и manual/semi-automated (docs/strict-viewer/reference-capture.md §3). Схема
    manifest отвергает чужое семейство: office-desktop с *-online — ошибка.

.PARAMETER Fixtures
    Корень фикстур, относительно которого разрешается source.path из корпуса.
    По умолчанию test-fixtures. Относительный путь ищется от текущего каталога,
    затем от корня репозитория.

.PARAMETER Corpus
    Corpus index — единственный источник истины. По умолчанию
    test-fixtures/strict-viewer/corpus.json.

.PARAMETER Out
    Каталог drop-ов, промежуточных файлов и отчёта. По умолчанию strict-viewer-refs-out.

.PARAMETER Dpi
    DPI растеризации. По умолчанию 144: схема manifest фиксирует dpi = 144, другое
    значение не пройдёт валидацию.

.PARAMETER BrowserVersion
    Полная версия Chromium для environment.browser.version. Обязателен, если в отборе
    есть режимы, снимаемые из PDF (docx.pdf, xlsx.print-preview): схема manifest требует
    непустую версию браузера для любого режима.

.PARAMETER FontPackage
    Имя версионированного шрифтового набора для environment.fonts.package.
    По умолчанию carlito-subset.

.PARAMETER FontVersion
    Версия шрифтового набора для environment.fonts.version. По умолчанию 1.104.

.PARAMETER Operator
    Оператор захвата для reference.operator. По умолчанию $env:USERNAME,
    затем имя машины.

.PARAMETER Filter
    Glob по id фикстуры (синтаксис -like). Пустой отбор — ошибка употребления.

.PARAMETER KeepPdf
    Не удалять рабочий каталог <Out>/work с промежуточными PDF и PNG.
    Сам reference/export.pdf в drop сохраняется всегда.

.PARAMETER IncludeWebModes
    Пометить docx.print-layout и xlsx.normal как «требует ручного захвата, не реализовано»
    вместо обычного Skip. Снимать их скрипт всё равно не умеет.

.NOTES
    Требования:
      * Windows 10/11 (COM-автоматизация);
      * установленные Microsoft Word и Excel (Word.Application/Excel.Application);
      * Poppler (pdftoppm);
      * Node.js и pnpm (импорт manifest через tools/visual-oracle);
      * PowerShell 5.1+.

    Перед запуском закройте Word и Excel: скрипт создаёт COM-экземпляры и завершает их
    через Quit(), чтобы не закрыть пользовательские документы.

    Запуск:
      powershell -ExecutionPolicy Bypass -File scripts\strict-viewer-references.ps1 `
        -BrowserVersion 131.0.6778.86 `
        -Out strict-viewer-refs-out

    Коды выхода:
      0 — прогон состоялся (часть фикстур может быть skipped);
      1 — ни одного drop не импортировано;
      2 — нет предпосылок (не Windows, нет Office/COM, pdftoppm, Node/pnpm, корпуса,
          шрифтового набора, не задан -BrowserVersion при наличии PDF-режимов)
          или неверное употребление.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\strict-viewer-references.ps1 `
      -BrowserVersion 131.0.6778.86 -Filter 'docx-*' -KeepPdf
#>
[CmdletBinding()]
param(
    [string]$Fixtures = 'test-fixtures',
    [string]$Corpus = 'test-fixtures/strict-viewer/corpus.json',
    [string]$Out = 'strict-viewer-refs-out',
    [int]$Dpi = 144,
    [string]$BrowserVersion,
    [string]$FontPackage = 'carlito-subset',
    [string]$FontVersion = '1.104',
    [string]$Operator,
    [string]$Filter,
    [switch]$KeepPdf,
    [switch]$IncludeWebModes
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# В PowerShell 7.3+ ненулевой код нативной команды может стать terminating error;
# импорт и pdftoppm возвращают коды, которые разбираются вручную.
if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
    $PSNativeCommandUseErrorActionPreference = $false
}
try {
    [Console]::OutputEncoding = [Text.Encoding]::UTF8
} catch {
    # Консоль без UTF-8 — не повод падать: отчёт пишется в файл.
}

# Предел одного PNG из baseline-policy §1: превышение — ошибка, а не повод уменьшать DPR.
$script:MaxPngBytes = 4 * 1024 * 1024
$script:RepoRoot = $null
$script:WordVersion = 'не использовался'
$script:ExcelVersion = 'не использовался'
$script:PdftoppmVersion = ''
$script:CorpusVersion = ''
$script:CorpusMatrixVersion = ''
$script:OsInfo = $null

# --- общие помощники --------------------------------------------------------

function Fail {
    param([string]$Message)
    Write-Host "ОШИБКА: $Message" -ForegroundColor Red
    exit 2
}

function Write-Step {
    param([string]$Message)
    Write-Host "==> $Message"
}

function Format-Cell {
    param($Text)
    if ($null -eq $Text) { return '' }
    $flat = ([string]$Text) -replace '\r?\n', ' '
    return $flat.Replace('|', '\|')
}

function Get-LastLine {
    param([string]$Text)
    if ([string]::IsNullOrWhiteSpace($Text)) { return '' }
    $lines = $Text -split '\r?\n'
    return $lines[$lines.Count - 1]
}

function Release-ComObject {
    param($ComObject)
    if ($null -eq $ComObject) { return }
    try {
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($ComObject)
    } catch {
        # Освобождение — best effort: исключение здесь не должно скрывать исходную ошибку.
    }
}

function Require-Command {
    param([string]$Name, [string]$Hint)
    $command = Get-Command -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $command) {
        Fail "не найдена команда $Name. $Hint"
    }
}

function Require-ComClass {
    param([string]$ProgId, [string]$Hint)
    $type = $null
    try {
        $type = [Type]::GetTypeFromProgID($ProgId, $true)
    } catch {
        Fail "COM-класс $ProgId недоступен. $Hint"
    }
    if ($null -eq $type) {
        Fail "COM-класс $ProgId не зарегистрирован. $Hint"
    }
}

function New-ProcessStartInfo {
    param([string]$FilePath, [string[]]$Arguments)
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $FilePath
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    # Аргументы приходят без кавычек: пробел внутри уже проставлен вызывающим
    # через «"…"» (пути репозитория бывают с пробелами).
    $info.Arguments = ($Arguments | ForEach-Object { $_ }) -join ' '
    return $info
}

# Нативная команда с файловыми потоками.
#
# `2>&1` в PowerShell 5.1 превращает каждую строку stderr в ErrorRecord, а при
# $ErrorActionPreference = 'Stop' это падение: `pdftoppm -v` пишет свой баннер
# в stderr даже на успехе, то есть документированный запуск умирал на старте.
# Здесь stderr уходит в файл и ошибкой PowerShell не становится ни в одной
# версии; код берётся из процесса, а не из $LASTEXITCODE.
function Invoke-NativeCapture {
    param([string]$FilePath, [string[]]$Arguments)
    $resolved = (Get-Command -Name $FilePath -ErrorAction Stop).Source
    $outFile = [IO.Path]::GetTempFileName()
    $errFile = [IO.Path]::GetTempFileName()
    $exitCode = 1
    try {
        $info = New-ProcessStartInfo -FilePath $resolved -Arguments $Arguments
        $stdout = New-Object System.IO.StreamWriter($outFile, $false, [Text.Encoding]::UTF8)
        $stderr = New-Object System.IO.StreamWriter($errFile, $false, [Text.Encoding]::UTF8)
        try {
            $info.RedirectStandardOutput = $true
            $info.RedirectStandardError = $true
            $process = [Diagnostics.Process]::Start($info)
            try {
                # Асинхронное копирование: синхронные ReadToEnd на двух потоках
                # одного процесса могут взаимно заблокироваться на большом выводе.
                $stdoutTask = $process.StandardOutput.ReadToEndAsync()
                $stderrTask = $process.StandardError.ReadToEndAsync()
                $process.WaitForExit()
                $stdout.Write($stdoutTask.Result)
                $stderr.Write($stderrTask.Result)
                $exitCode = $process.ExitCode
            } finally {
                $process.Dispose()
            }
        } finally {
            $stdout.Dispose()
            $stderr.Dispose()
        }
        return [pscustomobject]@{
            ExitCode = $exitCode
            StdOut   = [IO.File]::ReadAllText($outFile)
            StdErr   = [IO.File]::ReadAllText($errFile)
        }
    } finally {
        Remove-Item -LiteralPath $outFile, $errFile -Force -ErrorAction SilentlyContinue
    }
}

# Запуск через shim `.cmd` (pnpm — именно он) не передаёт ProcessStartInfo
# напрямую, поэтому вызов идёт через `cmd.exe /d /s /c`: кавычки внутри
# командной строки cmd разбирает сам.
function Invoke-NativeCaptureViaCmd {
    param([string]$Command, [string[]]$Arguments)
    $comspec = $env:ComSpec
    if ([string]::IsNullOrWhiteSpace($comspec)) { $comspec = 'cmd.exe' }
    return Invoke-NativeCapture -FilePath $comspec -Arguments (@('/d', '/s', '/c', $Command) + $Arguments)
}

# Путь обязан лежать внутри корня: иначе `Remove-Item -Recurse` по нему
# удалил бы чужой каталог. Id фикстуры и сегменты `source.path` приходят из
# корпуса, а валидацию корпуса делает другая программа — скрипт не вправе на
# неё рассчитывать.
function Assert-ChildPath {
    param([string]$Path, [string]$Root, [string]$What)
    # Оба разделителя, а не литерал: файл кодируется в UTF-8 без BOM, и
    # одиночная обратная косая черта в кавычках ломается на стыке разбора.
    $separators = [char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    # TrimEnd обязателен и у кандидата: GetFullPath на .NET сохраняет хвостовой
    # разделитель, и путь, равный корню с «/» на конце, прошёл бы проверку
    # как «внутренний», а Remove-Item по нему удалил бы сам корень.
    $full = [IO.Path]::GetFullPath($Path).TrimEnd($separators)
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd($separators)
    $inside = $full.StartsWith($rootFull + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
    if (-not $inside) {
        Fail "$What выходит за пределы $rootFull : $full"
    }
    return $full
}

function New-ComInstance {
    param([string]$ProgId, [string]$Hint)
    try {
        return New-Object -ComObject $ProgId
    } catch {
        Fail "не удалось создать COM-объект $ProgId. $Hint ($($_.Exception.Message))"
    }
}

function Resolve-RepoPath {
    param([string]$Path)
    if ([IO.Path]::IsPathRooted($Path)) {
        if (Test-Path -LiteralPath $Path) { return (Resolve-Path -LiteralPath $Path).Path }
        return $null
    }
    $candidate = Join-Path (Get-Location).Path $Path
    if (Test-Path -LiteralPath $candidate) { return (Resolve-Path -LiteralPath $candidate).Path }
    $candidate = Join-Path $script:RepoRoot $Path
    if (Test-Path -LiteralPath $candidate) { return (Resolve-Path -LiteralPath $candidate).Path }
    return $null
}

function Get-Sha256Hex {
    param([string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Write-Utf8NoBom {
    param([string]$Path, [string]$Text)
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($Path, $Text, $encoding)
}

function Get-OsInfo {
    $name = 'Windows'
    $version = [Environment]::OSVersion.Version.ToString()
    try {
        $os = Get-CimInstance -ClassName Win32_OperatingSystem -ErrorAction Stop
        if ($os.Caption) { $name = [string]$os.Caption }
        if ($os.Version) { $version = [string]$os.Version }
    } catch {
        # CIM может быть недоступен в урезанной среде — остаются значения .NET.
    }
    $architecture = $env:PROCESSOR_ARCHITECTURE
    if ([string]::IsNullOrWhiteSpace($architecture)) { $architecture = 'unknown' }
    return [pscustomobject]@{ Name = $name; Version = $version; Architecture = $architecture }
}

# --- PNG --------------------------------------------------------------------

function Get-PngHeader {
    param([string]$Path)
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 33) { return $null }
    $signature = @(0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)
    for ($i = 0; $i -lt 8; $i++) {
        if ($bytes[$i] -ne $signature[$i]) { return $null }
    }
    $chunkLength = ([int]$bytes[8] -shl 24) -bor ([int]$bytes[9] -shl 16) -bor ([int]$bytes[10] -shl 8) -bor [int]$bytes[11]
    if ($chunkLength -ne 13) { return $null }
    $chunkType = [Text.Encoding]::ASCII.GetString($bytes, 12, 4)
    if ($chunkType -ne 'IHDR') { return $null }
    $width = ([int]$bytes[16] -shl 24) -bor ([int]$bytes[17] -shl 16) -bor ([int]$bytes[18] -shl 8) -bor [int]$bytes[19]
    $height = ([int]$bytes[20] -shl 24) -bor ([int]$bytes[21] -shl 16) -bor ([int]$bytes[22] -shl 8) -bor [int]$bytes[23]
    return [pscustomobject]@{
        Width     = $width
        Height    = $height
        BitDepth  = [int]$bytes[24]
        ColorType = [int]$bytes[25]
        Bytes     = $bytes.Length
    }
}

function Test-ReferencePng {
    param([string]$Path)
    $header = Get-PngHeader -Path $Path
    if ($null -eq $header) {
        throw "PNG не имеет валидной сигнатуры и IHDR: $Path"
    }
    if ($header.BitDepth -ne 8) {
        throw "PNG должен быть 8-битным, получено bit depth $($header.BitDepth): $Path"
    }
    if ($header.ColorType -ne 2 -and $header.ColorType -ne 6) {
        throw "PNG должен быть RGB8 (colorType 2) или RGBA8 (colorType 6), получено colorType $($header.ColorType): $Path"
    }
    if ($header.Bytes -gt $script:MaxPngBytes) {
        throw "PNG больше 4 МиБ ($($header.Bytes) байт): $Path"
    }
    return $header
}

# --- COM-экспорт ------------------------------------------------------------

function Export-WordPdf {
    param($Word, [string]$InputPath, [string]$OutputPath)
    $document = $null
    try {
        # ConfirmConversions = $false, ReadOnly = $true: исходник не меняется.
        $document = $Word.Documents.Open($InputPath, $false, $true)
        # 17 = wdExportFormatPDF; Item по умолчанию wdExportDocumentContent —
        # PDF без разметки правок и комментариев, что соответствует view
        # revisions: accepted, comments: hidden.
        $document.ExportAsFixedFormat($OutputPath, 17)
    } finally {
        if ($null -ne $document) {
            try { $document.Close(0) } catch { }
            Release-ComObject $document
        }
    }
}

function Export-ExcelPdf {
    param($Excel, [string]$InputPath, [string]$OutputPath, [int]$SheetIndex)
    $workbook = $null
    $sheet = $null
    try {
        # UpdateLinks = 0, ReadOnly = $true: книга не меняется и не тянет внешние ссылки.
        $workbook = $Excel.Workbooks.Open($InputPath, 0, $true)
        $sheet = $workbook.Worksheets.Item($SheetIndex + 1)
        if ($sheet.Visible -ne -1) {
            throw "лист «$($sheet.Name)» скрыт; скрытые листы не снимаются (baseline-policy §2.1)"
        }
        # 0 = xlTypePDF, Quality 0 = xlQualityStandard, IncludeDocProperties = $true,
        # IgnorePrintAreas = $false: печатаются именно печатные страницы листа.
        $sheet.ExportAsFixedFormat(0, $OutputPath, 0, $true, $false)
        return [pscustomobject]@{ Index = $SheetIndex; Name = [string]$sheet.Name }
    } finally {
        if ($null -ne $sheet) { Release-ComObject $sheet }
        if ($null -ne $workbook) {
            try { $workbook.Close($false) } catch { }
            Release-ComObject $workbook
        }
    }
}

# --- растеризация -----------------------------------------------------------

function Get-PageNumber {
    param([string]$Path)
    if ($Path -match '-(\d+)\.png$') { return [int]$Matches[1] }
    return 0
}

function Invoke-Rasterize {
    param([string]$Pdf, [string]$Prefix, [int]$Dpi, [string]$LogPath)
    $dir = Split-Path -Parent $Prefix
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    # Пути идут в кавычках: вне PowerShell кавычки — наша забота.
    $run = Invoke-NativeCapture -FilePath 'pdftoppm' -Arguments @(
        '-png', '-r', [string]$Dpi, "`"$Pdf`"", "`"$Prefix`""
    )
    $code = $run.ExitCode
    $text = ($run.StdErr + $run.StdOut).Trim()
    if ($LogPath) {
        Set-Content -LiteralPath $LogPath -Value @($text) -Encoding UTF8
    }
    if ($code -ne 0) {
        throw "pdftoppm завершился с кодом ${code}: $(Get-LastLine $text)"
    }
    $files = @(Get-ChildItem -LiteralPath $dir -Filter '*.png' | Sort-Object { Get-PageNumber -Path $_.FullName })
    return $files
}

# --- capture.json -----------------------------------------------------------

function New-CaptureObject {
    param(
        [string]$Format,
        [string]$Mode,
        [string]$SourceRel,
        [string]$Application,
        [string]$ApplicationVersion,
        [string]$CapturedAt,
        [string]$OperatorName,
        [string]$BrowserVersion,
        [int]$Dpi,
        [string]$PdftoppmVersion,
        $FontAssets,
        [string]$FontPackage,
        [string]$FontVersion,
        [string[]]$ImagePaths,
        $Sheet
    )
    $os = $script:OsInfo
    $environment = [ordered]@{
        os                     = [ordered]@{ name = $os.Name; version = $os.Version; architecture = $os.Architecture }
        browser                = [ordered]@{ name = 'chromium'; version = $BrowserVersion }
        applicationZoomPercent = 100
        browserZoomPercent     = 100
        locale                 = 'ru-RU'
        timezone               = 'UTC'
        fonts                  = [ordered]@{ package = $FontPackage; version = $FontVersion; assets = $FontAssets }
    }
    if ($Mode -eq 'pdf') {
        # Схема запрещает viewportCss/deviceScaleFactor/colorScheme для pdf и требует rasterizer.
        $environment['rasterizer'] = [ordered]@{ name = 'pdftoppm'; version = $PdftoppmVersion; dpi = $Dpi }
    } else {
        # Схема требует viewportCss/deviceScaleFactor/colorScheme и запрещает rasterizer.
        $environment['viewportCss'] = [ordered]@{ width = 1440; height = 900 }
        $environment['deviceScaleFactor'] = 1
        $environment['colorScheme'] = 'light'
    }

    $captures = @()
    for ($i = 0; $i -lt $ImagePaths.Count; $i++) {
        $page = $i + 1
        if ($Format -eq 'docx') {
            $captures += [ordered]@{
                id         = "page-$page"
                kind       = 'docx-page'
                pageNumber = $page
                image      = $ImagePaths[$i]
            }
        } else {
            $captures += [ordered]@{
                id         = "page-$page"
                kind       = 'xlsx-print-page'
                pageNumber = $page
                sheet      = [ordered]@{ index = $Sheet.Index; name = $Sheet.Name }
                image      = $ImagePaths[$i]
            }
        }
    }

    if ($Format -eq 'docx') {
        $view = [ordered]@{ format = 'docx'; mode = $Mode; revisions = 'accepted'; comments = 'hidden' }
    } else {
        $view = [ordered]@{
            format       = 'xlsx'
            mode         = $Mode
            activeSheet  = [ordered]@{ index = $Sheet.Index; name = $Sheet.Name }
            hiddenSheets = 'exclude'
            showGridLines = $false
            showHeadings  = $false
        }
    }

    return [ordered]@{
        corpus      = [ordered]@{ version = $script:CorpusVersion; matrixVersion = $script:CorpusMatrixVersion }
        source      = [ordered]@{ path = $SourceRel }
        reference   = [ordered]@{
            provider           = 'office-desktop'
            application        = $Application
            applicationVersion = $ApplicationVersion
            capturedAt         = $CapturedAt
            captureMethod      = 'automated'
            operator           = $OperatorName
            exportedPdf        = [ordered]@{ uri = 'reference/export.pdf' }
        }
        environment = $environment
        view        = $view
        dynamicData = [ordered]@{
            dates         = 'fixed-value'
            fields        = 'cached-result'
            formulaCache  = [ordered]@{
                policy        = 'cached-values-only'
                dirty         = $false
                fullCalcOnLoad = $false
                forceFullCalc  = $false
            }
            externalLinks = 'cached-values-no-network'
        }
        captures    = $captures
    }
}

# --- импорт -----------------------------------------------------------------

function Invoke-ReferenceImport {
    param(
        [string]$DropDir,
        [string]$CorpusPath,
        [string]$ReferencesRoot,
        [string]$FixturesRoot,
        [string]$LogPath
    )
    # --references указывает на корень drop-ов: manifest пишется рядом с capture.json,
    # а image.uri разрешается внутри drop. Иначе импорт искал бы PNG в
    # test-fixtures/strict-viewer и падал бы на отсутствующих файлах.
    $run = Invoke-NativeCaptureViaCmd -Command 'pnpm' -Arguments @(
        'visual-oracle', 'reference', 'import',
        '--from', "`"$DropDir`"",
        '--corpus', "`"$CorpusPath`"",
        '--references', "`"$ReferencesRoot`"",
        '--fixtures', "`"$FixturesRoot`""
    )
    $code = $run.ExitCode
    $text = ($run.StdOut + $run.StdErr).Trim()
    if ($LogPath) {
        Set-Content -LiteralPath $LogPath -Value @($text) -Encoding UTF8
    }
    return [pscustomobject]@{ Code = $code; Output = $text }
}

# --- предпосылки ------------------------------------------------------------

if ($env:OS -ne 'Windows_NT') {
    Fail 'скрипт рассчитан на Windows: Microsoft Word/Excel COM. Запускайте на Windows с установленным Office.'
}
if ($Dpi -le 0) {
    Fail "DPI должен быть больше 0 (получено $Dpi)."
}

$script:RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
if (-not (Test-Path -LiteralPath (Join-Path $script:RepoRoot 'package.json'))) {
    Fail "не найден package.json в корне репозитория: $($script:RepoRoot)"
}

$corpusPath = Resolve-RepoPath -Path $Corpus
if ($null -eq $corpusPath) {
    Fail "не найден corpus index: $Corpus"
}
$fixturesRoot = Resolve-RepoPath -Path $Fixtures
if ($null -eq $fixturesRoot) {
    Fail "не найден каталог фикстур: $Fixtures"
}

try {
    $corpus = Get-Content -LiteralPath $corpusPath -Raw -Encoding UTF8 | ConvertFrom-Json
} catch {
    Fail "не удалось прочитать corpus index ${corpusPath}: $($_.Exception.Message)"
}
if ($null -eq $corpus -or $null -eq $corpus.fixtures) {
    Fail "corpus index $corpusPath не содержит fixtures."
}
$script:CorpusVersion = [string]$corpus.corpus.version
$script:CorpusMatrixVersion = [string]$corpus.corpus.matrixVersion
if ([string]::IsNullOrWhiteSpace($script:CorpusVersion) -or [string]::IsNullOrWhiteSpace($script:CorpusMatrixVersion)) {
    Fail "corpus index $corpusPath не содержит corpus.version/corpus.matrixVersion."
}

# Шрифтовой набор: версионированные Carlito-сабсеты из репозитория (ADR-0051).
$fontDir = Join-Path $script:RepoRoot 'crates/render/src/fonts'
if (-not (Test-Path -LiteralPath $fontDir)) {
    Fail "не найден шрифтовой набор: $fontDir (нужен для environment.fonts.assets)."
}
$fontFiles = @(
    @{ File = 'carlito-subset.ttf'; Style = 'normal'; Weight = 400 },
    @{ File = 'carlito-bold-subset.ttf'; Style = 'bold'; Weight = 700 },
    @{ File = 'carlito-italic-subset.ttf'; Style = 'italic'; Weight = 400 },
    @{ File = 'carlito-bolditalic-subset.ttf'; Style = 'bold-italic'; Weight = 700 }
)
$fontAssets = @()
foreach ($face in $fontFiles) {
    $fontPath = Join-Path $fontDir $face.File
    if (-not (Test-Path -LiteralPath $fontPath)) {
        Fail "не найден файл шрифта: $fontPath"
    }
    $fontAssets += [ordered]@{
        family = 'Carlito'
        style  = $face.Style
        weight = $face.Weight
        sha256 = (Get-Sha256Hex -Path $fontPath)
    }
}

# --- отбор фикстур и режимов ------------------------------------------------

$plans = New-Object System.Collections.Generic.List[object]
foreach ($fixture in @($corpus.fixtures)) {
    $fixtureId = [string]$fixture.id
    if ($Filter -and -not ($fixtureId -like $Filter)) { continue }
    $format = [string]$fixture.format
    # Id — это имя каталога под <Out>/work и <Out>/drop, и по нему делается
    # `Remove-Item -Recurse`. Slug из корпуса состоит из [a-z0-9-], поэтому
    # всё остальное — испорченный или подменённый корпус, а не опечатка.
    if ($fixtureId -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$' -or $fixtureId.Contains('..')) {
        Fail "id фикстуры «$fixtureId» не годится как имя каталога (ожидается slug из букв, цифр, '.', '_', '-')."
    }
    if ($format -ne 'docx' -and $format -ne 'xlsx') {
        Fail "фикстура «$fixtureId»: неизвестный формат «$format» (ожидается docx или xlsx)."
    }
    $sourceRel = [string]$fixture.source.path
    # `source.path` тоже становится сегментом пути: `/`, `..` и абсолютный путь
    # вывели бы чтение за корень фикстур.
    if ([string]::IsNullOrWhiteSpace($sourceRel) -or $sourceRel.Contains('..') -or
        [IO.Path]::IsPathRooted($sourceRel) -or $sourceRel.Contains(':') -or $sourceRel.Contains('\\')) {
        Fail "фикстура «$fixtureId»: source.path «$sourceRel» не является относительным путём."
    }
    $absolute = Join-Path $fixturesRoot ($sourceRel -replace '/', [IO.Path]::DirectorySeparatorChar)
    if (-not (Test-Path -LiteralPath $absolute) -and -not $sourceRel.Contains('/')) {
        $absolute = Join-Path $fixturesRoot $sourceRel
    }
    $sourcePath = Assert-ChildPath -Path $absolute -Root $fixturesRoot -What "source.path фикстуры «$fixtureId»"

    foreach ($modeValue in @($fixture.modes)) {
        $mode = [string]$modeValue
        $action = 'unknown'
        if ($format -eq 'docx' -and $mode -eq 'pdf') { $action = 'export' }
        elseif ($format -eq 'xlsx' -and $mode -eq 'print-preview') { $action = 'export' }
        elseif ($format -eq 'docx' -and $mode -eq 'print-layout') { $action = 'web' }
        elseif ($format -eq 'xlsx' -and $mode -eq 'normal') { $action = 'web' }
        if ($mode -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$') {
            Fail "фикстура «$fixtureId»: режим «$mode» не годится как имя каталога."
        }
        $plans.Add([pscustomobject]@{
            FixtureId  = $fixtureId
            Format     = $format
            Mode       = $mode
            SourceRel  = $sourceRel
            SourcePath = $sourcePath
            Action     = $action
        })
    }
}

if ($plans.Count -eq 0) {
    if ($Filter) {
        Fail "фильтр -Filter '$Filter' не совпал ни с одной фикстурой корпуса."
    }
    Fail "в корпусе $corpusPath нет фикстур."
}

$exportPlans = @($plans | Where-Object { $_.Action -eq 'export' })
$needWord = @($exportPlans | Where-Object { $_.Format -eq 'docx' }).Count -gt 0
$needExcel = @($exportPlans | Where-Object { $_.Format -eq 'xlsx' }).Count -gt 0

if ($exportPlans.Count -gt 0 -and [string]::IsNullOrWhiteSpace($BrowserVersion)) {
    $modes = ($exportPlans | ForEach-Object { "$($_.Format).$($_.Mode)" } | Sort-Object -Unique) -join ', '
    Fail "не задан -BrowserVersion: схема manifest требует environment.browser.version для режимов $modes."
}

Require-Command 'pdftoppm' 'Установите Poppler (pdftoppm) и добавьте его в PATH.'
Require-Command 'node' 'Установите Node.js 20+ и добавьте его в PATH.'
Require-Command 'pnpm' 'Установите pnpm 9 и добавьте его в PATH.'

# Poppler печатает баннер `pdftoppm -v` в stderr: читаем оба потока и берём
# первую непустую строку — на разных сборках она оказывается в любом из них.
$versionProbe = Invoke-NativeCapture -FilePath 'pdftoppm' -Arguments @('-v')
$pdftoppmLine = (($versionProbe.StdErr + "`n" + $versionProbe.StdOut) -split '\r?\n' |
    ForEach-Object { $_.Trim() } | Where-Object { $_ -ne '' } | Select-Object -First 1)
if ($pdftoppmLine -match 'version\s+([0-9][0-9A-Za-z.\-]*)') {
    $script:PdftoppmVersion = $Matches[1]
} else {
    $script:PdftoppmVersion = $pdftoppmLine
}
if ([string]::IsNullOrWhiteSpace($script:PdftoppmVersion)) {
    Fail 'не удалось определить версию pdftoppm (pdftoppm -v).'
}

if ($needWord) {
    Require-ComClass 'Word.Application' 'Установите Microsoft Word (нужна COM-автоматизация).'
}
if ($needExcel) {
    Require-ComClass 'Excel.Application' 'Установите Microsoft Excel (нужна COM-автоматизация).'
}

$script:OsInfo = Get-OsInfo
$operatorName = $Operator
if ([string]::IsNullOrWhiteSpace($operatorName)) { $operatorName = $env:USERNAME }
if ([string]::IsNullOrWhiteSpace($operatorName)) { $operatorName = [Environment]::MachineName }

# --- каталоги ---------------------------------------------------------------

if (-not [IO.Path]::IsPathRooted($Out)) {
    $Out = Join-Path (Get-Location).Path $Out
}
$Out = [IO.Path]::GetFullPath($Out)
$dropRoot = Join-Path $Out 'drop'
$workRoot = Join-Path $Out 'work'
$logDir = Join-Path $Out 'log'
foreach ($dir in @($Out, $dropRoot, $workRoot, $logDir)) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
}

if ($Dpi -ne 144) {
    Write-Host "ПРЕДУПРЕЖДЕНИЕ: схема manifest фиксирует dpi = 144; значение $Dpi не пройдёт валидацию." -ForegroundColor Yellow
}

# --- прогон -----------------------------------------------------------------

$results = New-Object System.Collections.Generic.List[object]
$word = $null
$excel = $null

try {
    if ($needWord) {
        $word = New-ComInstance 'Word.Application' 'Установите Microsoft Word (нужна COM-автоматизация).'
        try { $word.Visible = $false } catch { }
        try { $word.DisplayAlerts = 0 } catch { }
        try { $word.AutomationSecurity = 3 } catch { }
        $script:WordVersion = [string]$word.Version
    }
    if ($needExcel) {
        $excel = New-ComInstance 'Excel.Application' 'Установите Microsoft Excel (нужна COM-автоматизация).'
        try { $excel.Visible = $false } catch { }
        try { $excel.DisplayAlerts = $false } catch { }
        try { $excel.AutomationSecurity = 3 } catch { }
        $script:ExcelVersion = [string]$excel.Version
    }

    foreach ($plan in $plans) {
        $label = "$($plan.Format)/$($plan.FixtureId)/$($plan.Mode)"

        if ($plan.Action -eq 'web') {
            $reason = 'режим снимается с экрана веб-версии Word/Excel Online; PDF-экспорт его не даёт'
            if ($IncludeWebModes) {
                $reason = 'требует ручного захвата в веб-версии; скрипт этот режим не реализует'
            }
            Write-Step "$label — Skip ($reason)"
            $results.Add([pscustomobject]@{
                Format = $plan.Format; FixtureId = $plan.FixtureId; Mode = $plan.Mode
                Pages = 0; Status = 'skipped'; Reason = $reason; ManifestPath = ''
            })
            continue
        }
        if ($plan.Action -eq 'unknown') {
            $reason = "неизвестный режим «$($plan.Mode)» для формата $($plan.Format)"
            Write-Step "$label — Skip ($reason)"
            $results.Add([pscustomobject]@{
                Format = $plan.Format; FixtureId = $plan.FixtureId; Mode = $plan.Mode
                Pages = 0; Status = 'skipped'; Reason = $reason; ManifestPath = ''
            })
            continue
        }

        Write-Step $label
        $workDir = Join-Path (Join-Path (Join-Path $workRoot $plan.Format) $plan.FixtureId) $plan.Mode
        $dropDir = Join-Path (Join-Path (Join-Path $dropRoot $plan.Format) $plan.FixtureId) $plan.Mode
        $refDir = Join-Path $dropDir 'reference'
        $logBase = "$($plan.FixtureId)-$($plan.Mode)"
        # Проверка до удаления, а не после: проверять существование каталога
        # «на месте» уже поздно, если он указывает мимо <Out>.
        [void](Assert-ChildPath -Path $workDir -Root $workRoot -What 'рабочий каталог')
        [void](Assert-ChildPath -Path $dropDir -Root $dropRoot -What 'каталог drop')

        try {
            if (-not (Test-Path -LiteralPath $plan.SourcePath)) {
                throw "исходный документ не найден: $($plan.SourcePath)"
            }
            # Повторный прогон не должен оставлять страницы от прошлого захвата.
            foreach ($dir in @($workDir, $dropDir)) {
                if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
            }
            New-Item -ItemType Directory -Force -Path $workDir | Out-Null
            New-Item -ItemType Directory -Force -Path $refDir | Out-Null

            $pdfPath = Join-Path $workDir 'export.pdf'
            $sheet = $null
            if ($plan.Format -eq 'docx') {
                Export-WordPdf -Word $word -InputPath $plan.SourcePath -OutputPath $pdfPath
            } else {
                $sheet = Export-ExcelPdf -Excel $excel -InputPath $plan.SourcePath -OutputPath $pdfPath -SheetIndex 0
            }
            if (-not (Test-Path -LiteralPath $pdfPath)) {
                throw "Office не создал PDF: $pdfPath"
            }

            $prefix = Join-Path (Join-Path $workDir 'pages') 'page'
            $pageFiles = @(Invoke-Rasterize -Pdf $pdfPath -Prefix $prefix -Dpi $Dpi -LogPath (Join-Path $logDir "$logBase-pdftoppm.log"))
            if ($pageFiles.Count -eq 0) {
                throw 'pdftoppm не создал ни одной страницы'
            }

            $imagePaths = @()
            $pageNumber = 0
            foreach ($file in $pageFiles) {
                $pageNumber++
                [void](Test-ReferencePng -Path $file.FullName)
                if ($plan.Mode -eq 'pdf') {
                    $name = "page-$pageNumber.png"
                } else {
                    $name = "print-page-$pageNumber.png"
                }
                Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $refDir $name) -Force
                $imagePaths += "reference/$name"
            }

            Copy-Item -LiteralPath $pdfPath -Destination (Join-Path $refDir 'export.pdf') -Force

            $capturedAt = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
            if ($plan.Format -eq 'docx') {
                $application = 'word-desktop'
                $applicationVersion = $script:WordVersion
            } else {
                $application = 'excel-desktop'
                $applicationVersion = $script:ExcelVersion
            }

            $capture = New-CaptureObject `
                -Format $plan.Format `
                -Mode $plan.Mode `
                -SourceRel $plan.SourceRel `
                -Application $application `
                -ApplicationVersion $applicationVersion `
                -CapturedAt $capturedAt `
                -OperatorName $operatorName `
                -BrowserVersion $BrowserVersion `
                -Dpi $Dpi `
                -PdftoppmVersion $script:PdftoppmVersion `
                -FontAssets $fontAssets `
                -FontPackage $FontPackage `
                -FontVersion $FontVersion `
                -ImagePaths $imagePaths `
                -Sheet $sheet

            $captureJson = $capture | ConvertTo-Json -Depth 12
            Write-Utf8NoBom -Path (Join-Path $dropDir 'capture.json') -Text $captureJson

            $import = Invoke-ReferenceImport `
                -DropDir $dropDir `
                -CorpusPath $corpusPath `
                -ReferencesRoot $dropRoot `
                -FixturesRoot $fixturesRoot `
                -LogPath (Join-Path $logDir "$logBase-import.log")

            if ($import.Code -ne 0) {
                throw "импорт завершился с кодом $($import.Code): $(Get-LastLine $import.Output)"
            }

            $manifestPath = Join-Path $dropDir 'manifest.json'
            $results.Add([pscustomobject]@{
                Format = $plan.Format; FixtureId = $plan.FixtureId; Mode = $plan.Mode
                Pages = $pageNumber; Status = 'imported'; Reason = ''; ManifestPath = $manifestPath
            })
            Write-Host "    imported: $pageNumber стр., manifest $manifestPath"
        } catch {
            $reason = $_.Exception.Message
            $results.Add([pscustomobject]@{
                Format = $plan.Format; FixtureId = $plan.FixtureId; Mode = $plan.Mode
                Pages = 0; Status = 'failed'; Reason = $reason; ManifestPath = ''
            })
            Write-Host "    failed: $reason" -ForegroundColor Yellow
        }
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
}

# --- отчёт ------------------------------------------------------------------

$reportPath = Join-Path $Out 'report.md'
$importedRows = @($results | Where-Object { $_.Status -eq 'imported' })
$skippedRows = @($results | Where-Object { $_.Status -eq 'skipped' })
$failedRows = @($results | Where-Object { $_.Status -eq 'failed' })
$totalPages = 0
foreach ($row in $importedRows) { $totalPages += [int]$row.Pages }

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('# Эталоны strict-viewer из локального Microsoft Office')
$lines.Add('')
$lines.Add('> **Оговорка о политике.** Скрипт снимает эталон локальным Microsoft Word/Excel через COM —')
$lines.Add('> способ захвата разрешён решением `accept-capture-automation` (`docs/adr/0056-capture-automation-com.md`,')
$lines.Add('> `baseline-policy.md` v1.1.0 §1 и §3 п.5). Reference отсюда — **кандидат, а не accepted baseline**:')
$lines.Add('> принятие по-прежнему требует baseline-only PR и двух approvals (§5).')
$lines.Add('')
$lines.Add("- Дата: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')")
$lines.Add("- Машина: $env:COMPUTERNAME")
$lines.Add("- Оператор: $operatorName")
$lines.Add("- ОС: $($script:OsInfo.Name) $($script:OsInfo.Version) ($($script:OsInfo.Architecture))")
$lines.Add("- Microsoft Word: $($script:WordVersion)")
$lines.Add("- Microsoft Excel: $($script:ExcelVersion)")
$lines.Add("- Poppler: $($script:PdftoppmVersion)")
$lines.Add("- DPI: $Dpi, растеризация ``pdftoppm -png -r $Dpi`` (без ``-gray``)")
$lines.Add("- Корпус: ``$corpusPath`` (version $($script:CorpusVersion), matrixVersion $($script:CorpusMatrixVersion))")
$lines.Add("- Drop: ``$dropRoot``")
$lines.Add("- Шрифтовой набор: $FontPackage $FontVersion ($($fontAssets.Count) файла, SHA-256 в capture.json)")
if ($Filter) {
    $lines.Add("- Фильтр: ``$Filter``")
}
if ($IncludeWebModes) {
    $lines.Add('- Веб-режимы помечены через `-IncludeWebModes`')
}
$lines.Add('')
$lines.Add('| Формат | Фикстура | Режим | Страниц | Статус | Причина |')
$lines.Add('| --- | --- | --- | ---: | --- | --- |')
foreach ($row in $results) {
    $lines.Add("| $(Format-Cell $row.Format) | $(Format-Cell $row.FixtureId) | $(Format-Cell $row.Mode) | $($row.Pages) | $(Format-Cell $row.Status) | $(Format-Cell $row.Reason) |")
}
$lines.Add('')
$lines.Add('## Итоги')
$lines.Add('')
$lines.Add("Импортировано: $($importedRows.Count), пропущено: $($skippedRows.Count), с ошибкой: $($failedRows.Count).")
$lines.Add("Страниц в импортированных drop-ах: $totalPages.")
$lines.Add('')
$lines.Add('## Ограничения')
$lines.Add('')
$lines.Add('- `docx.print-layout` и `xlsx.normal` снимаются с экрана веб-версии Word/Excel Online и из PDF-экспорта не получаются. Скрипт их пропускает; эквивалент не выдумывается.')
$lines.Add('- Эталон снимает локальный Word/Excel через COM — разрешённый способ захвата (`accept-capture-automation`, `baseline-policy.md` v1.1.0 §1).')
$lines.Add('- `revisions: accepted`, `comments: hidden` достигаются экспортом без разметки (`wdExportDocumentContent`): правки не «принимаются» физически, а не попадают в PDF.')
$lines.Add('- В `environment.fonts` записан версионированный набор Carlito из репозитория; фактический рендер Word/Excel использует системные шрифты. Расхождение фиксируется как ограничение кандидата.')
$lines.Add('- XLSX снимается только с листа с индексом 0 (первый лист книги); скрытый лист — ошибка фикстуры.')
$lines.Add('- `showGridLines: false`, `showHeadings: false` объявлены для print-preview; если в книге включена печать сетки или заголовков, объявление разойдётся с изображением.')
$lines.Add('- Импорт вызывается с `--references <Out>/drop`, поэтому manifest пишется рядом с `capture.json`, а drop самодостаточен. Перенос drop в `test-fixtures/strict-viewer` — отдельный baseline-only PR.')
$lines.Add('- Reference — кандидат: принятие требует baseline-only PR и двух approvals (baseline-policy §5).')
$lines.Add('')
$lines.Add('## Артефакты')
$lines.Add('')
$lines.Add("- ``$dropRoot`` — drop-ы ``<format>/<fixture-id>/<mode>/`` с ``capture.json``, ``manifest.json`` и ``reference/``.")
$lines.Add("- ``$logDir`` — stdout/stderr pdftoppm и импорта.")
if ($KeepPdf) {
    $lines.Add("- ``$workRoot`` — промежуточные PDF и PNG (``-KeepPdf``).")
}
$lines.Add('')

Write-Utf8NoBom -Path $reportPath -Text ($lines -join [Environment]::NewLine)

if (-not $KeepPdf -and (Test-Path -LiteralPath $workRoot)) {
    Remove-Item -LiteralPath $workRoot -Recurse -Force
}

Write-Host ''
Write-Host ($lines -join [Environment]::NewLine)
Write-Host ''
Write-Host "Отчёт: $reportPath"

if ($importedRows.Count -eq 0) {
    Write-Host "ПРЕДУПРЕЖДЕНИЕ: ни одного drop не импортировано; смотрите $reportPath" -ForegroundColor Yellow
    exit 1
}
exit 0
