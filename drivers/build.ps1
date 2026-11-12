# Builds the driver blocks of the library: drivers\<name> (Rust) -> library\<name>\block.wasm.
# Usage: .\drivers\build.ps1 [name ...]   (needs: rustup target add wasm32-unknown-unknown)
# Flags: MVP features only, a 4 KB stack (the window of a driver is its stack plus its data),
# exports step/init/__heap_base (help: «Драйверы железа на WASM»).
param([string[]]$Names)
$here = $PSScriptRoot
$library = Join-Path (Split-Path $here) 'library'
if (-not $Names) {
    $Names = Get-ChildItem $here -Directory | Where-Object { Test-Path (Join-Path $_.FullName 'Cargo.toml') } |
        ForEach-Object Name
}
$env:RUSTFLAGS = '-C target-cpu=mvp -C link-arg=-zstack-size=4096 -C link-arg=--export=__heap_base'
try {
    foreach ($name in $Names) {
        Push-Location (Join-Path $here $name)
        cargo build --release --target wasm32-unknown-unknown
        $ok = $LASTEXITCODE -eq 0
        Pop-Location
        if (-not $ok) { throw "драйвер $name не собран" }
        $wasm = Join-Path $here "$name\target\wasm32-unknown-unknown\release\$name.wasm"
        Copy-Item $wasm (Join-Path $library "$name\block.wasm") -Force
        Write-Host "$name -> library\$name\block.wasm ($((Get-Item $wasm).Length) байт)"
    }
} finally {
    $env:RUSTFLAGS = $null
}
