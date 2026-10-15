$port = New-Object System.IO.Ports.SerialPort 'COM4', 115200
$port.ReadTimeout = 6000
try {
    $port.Open()
    Write-Host "Reading serial log stream..."
    for ($i = 0; $i -lt 100; $i++) {
        try {
            $line = $port.ReadLine()
            Write-Host $line
        } catch [System.TimeoutException] {
            Write-Host "(idle timeout)"
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
