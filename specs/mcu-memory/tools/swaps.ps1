# Acceptance check of specs/mcu-memory on a board with the master: next to the modules already on
# the board it uploads one more module (in chunks, as the editor does) N times in a row and checks
# that the free heap does not go down. Prints `free`/`largest`/`min` after each upload.
# Usage: swaps.ps1 -Port COM5 -Wasm $env:TEMP\mods\counter.wasm [-Name counter] [-Count 20] [-Allow -]
param(
    [string]$Port = 'COM5',
    [string]$Wasm = ($env:TEMP + '\mods\counter.wasm'),
    [string]$Name = '',
    [int]$Count = 20,
    [string]$Allow = '-',
    [int]$Slack = 1024  # bytes the heap may differ by (Wi-Fi and MQTT buffers come and go)
)
if (-not $Name) { $Name = [IO.Path]::GetFileNameWithoutExtension($Wasm) }

$table = for ($i = 0; $i -lt 256; $i++) {
    $c = [uint64]$i
    for ($k = 0; $k -lt 8; $k++) { if ($c -band 1) { $c = [uint64]3988292384 -bxor ($c -shr 1) } else { $c = $c -shr 1 } }
    $c
}
function Crc32([byte[]]$b) {
    $c = [uint64]4294967295
    foreach ($x in $b) { $c = $table[[int](($c -bxor $x) -band 255)] -bxor ($c -shr 8) }
    return [uint64]($c -bxor 4294967295)
}

$p = New-Object System.IO.Ports.SerialPort $Port, 115200
$p.ReadTimeout = 5000; $p.NewLine = "`n"; $p.DtrEnable = $false; $p.RtsEnable = $false
$p.Open(); Start-Sleep -Milliseconds 300; $p.DiscardInBuffer()

function Heap {
    $p.WriteLine('list')
    while ($true) {
        $l = $p.ReadLine().Trim()
        if ($l -match '^free=(\d+) largest=(\d+) min=(\d+)') { $h = @([int]$Matches[1], [int]$Matches[2], [int]$Matches[3]) }
        if ($l -eq 'end') { return $h }
    }
}
$bytes = [IO.File]::ReadAllBytes($Wasm)
$crc = Crc32 $bytes
function Put {
    $p.WriteLine(("put {0} {1} {2:x8} {3}" -f $Name, $bytes.Length, $crc, $Allow))
    $first = $p.ReadLine().Trim()
    if (-not $first.StartsWith('ready ')) { return $first }
    $chunk = [int]$first.Substring(6)
    for ($i = 0; $i -lt $bytes.Length; $i += $chunk) {
        $p.Write($bytes, $i, [Math]::Min($chunk, $bytes.Length - $i))
        $l = $p.ReadLine().Trim()
        if (-not $l.StartsWith('ack ')) { return $l }
    }
    while ($true) { $l = $p.ReadLine().Trim(); if ($l -eq 'ok' -or $l.StartsWith('err')) { return $l } }
}

# The first upload puts the module on the board; the heap is measured after it.
$r = Put
if ($r -ne 'ok') { "first upload: $r"; $p.Close(); exit 1 }
Start-Sleep -Milliseconds 500
$start = Heap
"start: free=$($start[0]) largest=$($start[1]) min=$($start[2])"
$failed = 0
for ($n = 1; $n -le $Count; $n++) {
    $r = Put
    if ($r -ne 'ok') { $failed++ }
    Start-Sleep -Milliseconds 300
    $h = Heap
    "$n`: $r free=$($h[0]) largest=$($h[1]) min=$($h[2])"
}
$end = Heap
$p.Close()
$ok = ($failed -eq 0) -and ($end[0] -ge $start[0] - $Slack)
"uploads: $Count, failed: $failed, free $($start[0]) -> $($end[0])"
if ($ok) { 'OK' } else { 'FAIL'; exit 1 }
