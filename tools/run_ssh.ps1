$env:SSH_ASKPASS = "$PSScriptRoot\askpass.bat"
$env:SSH_ASKPASS_REQUIRE = "force"
$env:DISPLAY = "dummy:0"

Write-Host "Running OpenSSH with ASKPASS: $env:SSH_ASKPASS"

& ssh.exe -o StrictHostKeyChecking=no -o UserKnownHostsFile=NUL -o PreferredAuthentications=password youareme@192.168.100.77 "uptime; uname -a"
