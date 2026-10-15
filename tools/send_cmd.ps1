$port = New-Object System.IO.Ports.SerialPort 'COM4', 115200
$port.ReadTimeout = 4000
try {
    $port.Open()
    Start-Sleep -Milliseconds 300
    Write-Host "Sending wifi status command..."
    $port.WriteLine("wifi status")
    Start-Sleep -Milliseconds 800
    for ($i = 0; $i -lt 50; $i++) {
        try {
            $line = $port.ReadLine()
            Write-Host $line
        } catch [System.TimeoutException] {
            break
        }
    }
} catch {
    Write-Host "Error: $_"
} finally {
    if ($port.IsOpen) {
        $port.Close()
    }
}
