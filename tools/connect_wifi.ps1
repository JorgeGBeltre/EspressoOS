$port = New-Object System.IO.Ports.SerialPort "COM4", 115200, "None", 8, "One"
$port.DtrEnable = $false
$port.RtsEnable = $false
$port.ReadTimeout = 1000
$port.WriteTimeout = 1000

try {
    $port.Open()
    Start-Sleep -Milliseconds 200
    $port.DiscardInBuffer()

    $cmd = 'wifi connect "Familia beltre" 8099648946'
    Write-Host "Sending: $cmd"
    $port.Write($cmd + "`r`n")

    $deadline = (Get-Date).AddMilliseconds(8000)
    while ((Get-Date) -lt $deadline) {
        if ($port.BytesToRead -gt 0) {
            $buf = $port.ReadExisting()
            Write-Host -NoNewline $buf
        } else {
            Start-Sleep -Milliseconds 50
        }
    }
    Write-Host ""
} finally {
    if ($port.IsOpen) {
        $port.Close()
    }
}
