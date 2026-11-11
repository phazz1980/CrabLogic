# Builds library\bmp280\block.wasm from the Rust or the C source.
# Usage: .\build.ps1 rust   (needs: rustup target add wasm32-unknown-unknown)
#        .\build.ps1 c      (needs clang with the wasm32 target, or zig: set $env:ZIG to zig.exe)
# Flags: MVP features only (the prebuilt Rust `core` may still use memory.copy/fill: allowed),
# a 4 KB stack (the window of the driver is its stack plus its data), exports step/init/__heap_base.
param([ValidateSet('rust', 'c')][string]$Lang = 'rust')
$here = $PSScriptRoot
$out = "$here\library\bmp280\block.wasm"
if ($Lang -eq 'rust') {
    Push-Location "$here\rust"
    $env:RUSTFLAGS = '-C target-cpu=mvp -C link-arg=-zstack-size=4096 -C link-arg=--export=__heap_base'
    cargo build --release --target wasm32-unknown-unknown
    $env:RUSTFLAGS = $null
    Pop-Location
    Copy-Item "$here\rust\target\wasm32-unknown-unknown\release\bmp280.wasm" $out -Force
} else {
    $flags = @('-O2', '-mcpu=mvp', '-nostdlib', '-Wl,--no-entry',
        '-Wl,-z,stack-size=4096', '-Wl,--export=step', '-Wl,--export=init',
        '-Wl,--export=__heap_base', '-o', $out, "$here\c\bmp280.c")
    if ($env:ZIG) {
        & $env:ZIG cc --target=wasm32-freestanding @flags
    } else {
        clang --target=wasm32 @flags
    }
}
