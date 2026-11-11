# Packs the Xtensa-capable wamrc (built by build-wamrc.cmd) for a CrabLogic GitHub release:
# wamrc.exe plus the VC++ runtime DLLs it needs (app-local, so no VC++ Redistributable install).
# Prints the SHA-256 to pin in crates/crab-build/src/wamrc.rs (WAMRC_ESP_ZIP_SHA256).
# Usage: powershell -File firmware\master-rs\tools\pack-wamrc.ps1 [-Out <folder>]
param([string]$Out = (Join-Path $PSScriptRoot "..\..\..\target\wamrc-esp"))
$ErrorActionPreference = "Stop"
$version = "2.4.5"
$exe = Join-Path $PSScriptRoot "..\.deps\wamrc-esp\wamrc.exe"
if (-not (Test-Path $exe)) { throw "no ${exe}: run build-wamrc.cmd first" }
$crt = Get-ChildItem "C:\Program Files (x86)\Microsoft Visual Studio\2022\*\VC\Redist\MSVC\*\x64\Microsoft.VC143.CRT" |
    Sort-Object FullName | Select-Object -Last 1
if (-not $crt) { throw "VC++ redistributable folder (Microsoft.VC143.CRT) not found" }
$stage = Join-Path $Out "stage"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item $exe $stage
foreach ($dll in "msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll") {
    Copy-Item (Join-Path $crt.FullName $dll) $stage
}
$zip = Join-Path $Out "wamrc-esp-$version-x86_64-windows.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
$item = Get-Item $zip
"{0}  {1} bytes" -f $item.FullName, $item.Length
"SHA-256: " + (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
