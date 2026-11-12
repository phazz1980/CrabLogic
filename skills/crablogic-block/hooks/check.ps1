# Checks CrabLogic user blocks with `crablogic block check` (and the test project with `check`).
#
#   check.ps1 <folder or file>   manual: checks the block that folder or file belongs to
#   check.ps1                    Claude Code PostToolUse hook: the edited file comes as JSON on stdin
#   check.ps1 -Stop              Claude Code Stop hook: every block changed in the last hours
#
# Exit code 2 with the errors on stderr makes Claude Code show them to the model (PostToolUse) or
# keeps it working until they are fixed (Stop). Paths are relative to the program folder
# (three levels up: <program>\skills\crablogic-block\hooks).
param([string]$Path, [switch]$Stop)

$ErrorActionPreference = 'Stop'
# Claude Code sends UTF-8 JSON; paths may be Cyrillic («Мои блоки»).
[Console]::InputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$library = Join-Path $root 'library'
$exe = $env:CRABLOGIC_EXE
if (-not $exe) {
    # Next to the editor in the program folder; in the source repository — the debug build.
    $exe = @('crablogic.exe', 'target\debug\crablogic.exe', 'target\release\crablogic.exe') |
        ForEach-Object { Join-Path $root $_ } | Where-Object { Test-Path $_ } | Select-Object -First 1
}

function Read-HookInput {
    $text = [Console]::In.ReadToEnd()
    if ([string]::IsNullOrWhiteSpace($text)) { return $null }
    try { return $text | ConvertFrom-Json } catch { return $null }
}

# The block folder (the one with block.toml) a file or folder belongs to, at most 3 levels up.
function Find-BlockDir([string]$p) {
    if (-not $p -or -not (Test-Path $p)) { return $null }
    $dir = (Resolve-Path $p).Path
    if (Test-Path $dir -PathType Leaf) { $dir = Split-Path $dir }
    for ($i = 0; $i -lt 4 -and $dir; $i++) {
        if (Test-Path (Join-Path $dir 'block.toml')) { return $dir }
        if ($dir.TrimEnd('\') -eq $root.TrimEnd('\')) { break }
        $dir = Split-Path $dir
    }
    return $null
}

# Runs crablogic; returns @(ok, text).
function Invoke-Crab([string[]]$cmd) {
    # Native stderr must not turn into PowerShell errors (Windows PowerShell 5.1).
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $out = & $exe --library $library @cmd 2>&1 | ForEach-Object { "$_" }
    $code = $LASTEXITCODE
    $ErrorActionPreference = $old
    return @(($code -eq 0), ($out -join "`n"))
}

# Checks one block folder and, for a block inside <project>\library\<type>, that project.
function Test-Block([string]$dir) {
    $problems = @()
    $r = Invoke-Crab @('block', 'check', $dir)
    # A broken block breaks its project too: report the block alone.
    if (-not $r[0]) { return @("crablogic block check $dir`n$($r[1])") }
    $lib = Split-Path $dir
    $project = Split-Path $lib
    if ((Split-Path $lib -Leaf) -eq 'library' -and (Test-Path (Join-Path $project 'project.toml'))) {
        $r = Invoke-Crab @('check', $project)
        if (-not $r[0]) { $problems += "crablogic check $project`n$($r[1])" }
    }
    return $problems
}

function Fail([string[]]$problems, [string]$tail) {
    [Console]::Error.WriteLine(($problems -join "`n`n") + "`n`n" + $tail)
    exit 2
}

if (-not $exe) {
    [Console]::Error.WriteLine("crablogic.exe не найден в $root; задайте путь в переменной CRABLOGIC_EXE.")
    exit 1
}

if ($Stop) {
    $hook = Read-HookInput
    # Claude Code is already continuing because of this hook: do not hold it forever.
    if ($hook -and $hook.stop_hook_active) { exit 0 }
    $since = (Get-Date).AddHours(-6)
    $dirs = foreach ($base in 'blocks_src', 'user_library') {
        $b = Join-Path $root $base
        if (Test-Path $b) {
            Get-ChildItem $b -Recurse -Filter block.toml -File |
                Where-Object { $_.FullName -notmatch '\\target\\' } |
                Where-Object {
                    $d = $_.Directory
                    @($d.GetFiles()) | Where-Object { $_.LastWriteTime -gt $since }
                } | ForEach-Object { $_.Directory.FullName }
        }
    }
    $problems = @($dirs | Select-Object -Unique | ForEach-Object { Test-Block $_ })
    if ($problems.Count) {
        Fail $problems 'Блоки с ошибками: исправьте по skills/crablogic-block/reference/errors.md и пройдите checklist.md.'
    }
    exit 0
}

if (-not $Path) {
    $hook = Read-HookInput
    if ($hook -and $hook.tool_input) { $Path = $hook.tool_input.file_path }
    if (-not $Path) { exit 0 }
    $leaf = Split-Path $Path -Leaf
    # The test project of a block: check it as a whole.
    if ($leaf -match '^(logic|project)\.toml$') {
        $project = Split-Path $Path
        if (-not (Test-Path (Join-Path $project 'project.toml'))) { exit 0 }
        $r = Invoke-Crab @('check', $project)
        if (-not $r[0]) {
            Fail @("crablogic check $project`n$($r[1])") 'Исправьте проект и сохраните файл снова.'
        }
        exit 0
    }
    # Otherwise only files of a block: its description, code and docs.
    if ($leaf -notmatch '^(block\.toml|block\.wasm|docs\.md)$') { exit 0 }
}

$dir = Find-BlockDir $Path
if (-not $dir) {
    if ($PSBoundParameters.ContainsKey('Path')) {
        [Console]::Error.WriteLine("$Path — не папка блока: рядом нет block.toml.")
        exit 1
    }
    exit 0
}
$problems = @(Test-Block $dir)
if ($problems.Count) {
    Fail $problems 'Исправьте по skills/crablogic-block/reference/errors.md и сохраните файл снова.'
}
if ($PSBoundParameters.ContainsKey('Path')) { Write-Output "Блок $dir — ошибок нет." }
exit 0
