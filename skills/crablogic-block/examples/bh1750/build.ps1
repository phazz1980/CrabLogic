# Builds src/lib.rs into project\library\<name>\block.wasm, <name> = this folder = crate = block type (needs: rustup target add wasm32-unknown-unknown).
# Flags are the ones of drivers\build.ps1: MVP features only, a 4 KB stack, exported __heap_base.
$here = $PSScriptRoot
$name = Split-Path $here -Leaf
$env:RUSTFLAGS = '-C target-cpu=mvp -C link-arg=-zstack-size=4096 -C link-arg=--export=__heap_base'
try {
    Push-Location $here
    cargo build --release --target wasm32-unknown-unknown
    $ok = $LASTEXITCODE -eq 0
    Pop-Location
    if (-not $ok) { throw "${name}: build failed" }
    $wasm = Join-Path $here "target\wasm32-unknown-unknown\release\$name.wasm"
    $dest = Join-Path $here "project\library\$name\block.wasm"
    Copy-Item $wasm $dest -Force
    Write-Host "$name -> $dest ($((Get-Item $dest).Length) bytes)"
} finally {
    $env:RUSTFLAGS = $null
}
