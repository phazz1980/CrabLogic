# Puts the AI instructions into a program folder (dist\CrabLogic or any test folder):
#   skills\ (without build output and skills\dist), and from skills\dist: AGENTS.md, CLAUDE.md,
#   .claude\settings.json (hooks), .cursor\rules\. Creates blocks_src\ for block sources.
# The user's own .claude\settings.local.json and blocks_src\ contents are left alone.
# Usage: .\scripts\copy-skills.ps1 -Dest dist\CrabLogic
param([Parameter(Mandatory)][string]$Dest)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$skills = Join-Path $root 'skills'
New-Item -ItemType Directory -Force $Dest | Out-Null

& robocopy $skills (Join-Path $Dest 'skills') /MIR /XD target .crab dist /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "не удалось скопировать skills (robocopy $LASTEXITCODE)" }

$extra = Join-Path $skills 'dist'
foreach ($file in 'AGENTS.md', 'CLAUDE.md') {
    Copy-Item (Join-Path $extra $file) (Join-Path $Dest $file) -Force
}
foreach ($dir in '.claude', '.cursor') {
    # Not mirrored: files the user added there (settings.local.json, own rules) stay.
    & robocopy (Join-Path $extra $dir) (Join-Path $Dest $dir) /E /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "не удалось скопировать $dir (robocopy $LASTEXITCODE)" }
}
New-Item -ItemType Directory -Force (Join-Path $Dest 'blocks_src') | Out-Null
$global:LASTEXITCODE = 0
Write-Host "Инструкции для ИИ -> $Dest (skills, AGENTS.md, CLAUDE.md, .claude, .cursor, blocks_src)"
