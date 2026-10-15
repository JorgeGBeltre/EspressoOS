$env:SSH_ASKPASS = "$PSScriptRoot\askpass.bat"
$env:SSH_ASKPASS_REQUIRE = "force"
$env:DISPLAY = "dummy:0"

Write-Host "--- Running 'ifconfig; free; ps' over SSH ---"
& ssh.exe -o StrictHostKeyChecking=no -o UserKnownHostsFile=NUL -o PreferredAuthentications=password youareme@192.168.100.77 "ifconfig; free; ps"
