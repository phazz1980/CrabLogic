# Talks to the master over its USB console, for checks by hand (the editor does the same through
# crab-build). Usage:
#   tools\master.ps1 COM5 list
#   tools\master.ps1 COM5 "tick blink 3 500"
#   tools\master.ps1 COM5 put blink path\to\blink.aot gpio_write,millis [mem bytes]
# Several commands may follow each other, separated by "then" ("wait <ms>" pauses):
#   tools\master.ps1 COM5 put blink blink.aot gpio_write,millis then wait 1000 then list then info
param(
    [Parameter(Mandatory = $true)][string]$Port,
    [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)][object[]]$Rest
)
$ErrorActionPreference = 'Stop'

function Get-Crc32([byte[]]$Data) {
    [uint32]$crc = 4294967295
    foreach ($b in $Data) {
        $crc = $crc -bxor $b
        for ($i = 0; $i -lt 8; $i++) {
            if ($crc -band 1) { $crc = ($crc -shr 1) -bxor 3988292384 } else { $crc = $crc -shr 1 }
        }
    }
    return $crc -bxor 4294967295
}

$serial = New-Object System.IO.Ports.SerialPort $Port, 115200
$serial.ReadTimeout = 8000
$serial.WriteTimeout = 8000
$serial.NewLine = "`n"
# The master answers in UTF-8 (module errors are in Russian).
$serial.Encoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
# Without DTR the USB console of an ESP32-C3 does not send.
$serial.DtrEnable = $true
$serial.Open()

function Read-Answer([string]$Until) {
    while ($true) {
        $line = $serial.ReadLine().Trim()
        if ($line -eq '') { continue }
        Write-Host $line
        if ($line -match $Until) { return $line }
    }
}

function Send-Line([string]$Line) {
    Write-Host "> $Line"
    $serial.Write("$Line`n")
}

function Invoke-Put([string]$Name, [string]$File, [string]$Allow, [string]$Mem) {
    [byte[]]$data = [System.IO.File]::ReadAllBytes((Resolve-Path $File))
    $crc = '{0:x8}' -f (Get-Crc32 $data)
    $header = "put $Name $($data.Length) $crc $Allow"
    if ($Mem) { $header += " mem=$Mem" }
    Send-Line $header
    $ready = Read-Answer '^(ready|err)'
    if ($ready -notmatch '^ready (\d+)') { return }
    $chunk = [int]$Matches[1]
    for ($off = 0; $off -lt $data.Length; $off += $chunk) {
        $n = [Math]::Min($chunk, $data.Length - $off)
        $serial.Write($data, $off, $n)
        $ack = $serial.ReadLine().Trim()
        if ($ack -notmatch '^ack ') { Write-Host $ack; return }
    }
    $null = Read-Answer '^(ok|err)'
}

try {
    Start-Sleep -Milliseconds 300
    $serial.DiscardInBuffer()
    $commands = @()
    $current = @()
    foreach ($item in $Rest) {
        # PowerShell turns a,b into an array.
        $word = $item -join ','
        if ($word -eq 'then') { $commands += , $current; $current = @() } else { $current += $word }
    }
    $commands += , $current
    foreach ($c in $commands) {
        if ($c.Count -eq 0) { continue }
        if ($c[0] -eq 'put') {
            Invoke-Put $c[1] $c[2] $c[3] $(if ($c.Count -gt 4) { $c[4] } else { '' })
        } elseif ($c[0] -eq 'wait') {
            Start-Sleep -Milliseconds ([int]$c[1])
        } else {
            Send-Line ($c -join ' ')
            $null = Read-Answer '^(ok|err|end)'
        }
    }
} finally {
    $serial.Close()
}
