# Frees disk space taken by regenerable build caches. Run from anywhere:
#   powershell -File scripts\clean-cache.ps1
# Removes: target/debug (old test binaries pile up there), .m/debug (master debug build),
# .embuild/espressif/dist (downloaded ESP-IDF tool archives, already extracted).
# Keeps release builds, toolchains and the AOT compiler (slow to rebuild).

$root = Split-Path -Parent $PSScriptRoot
$paths = @(
    "target\debug",
    ".m\debug",
    ".embuild\espressif\dist"
)

foreach ($p in $paths) {
    $full = Join-Path $root $p
    if (Test-Path $full) {
        $size = (Get-ChildItem $full -Recurse -Force -ErrorAction SilentlyContinue |
            Measure-Object Length -Sum).Sum
        Remove-Item $full -Recurse -Force
        "{0,8:N1} MB freed: {1}" -f ($size / 1MB), $p
    }
}
