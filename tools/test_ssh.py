import paramiko
import time
import socket

HOST = "192.168.100.77"
PORT = 22
USER = "youareme"
PASS = "851963Y@#"

print(f"Connecting to {HOST}:{PORT}...")

# First test raw banner
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.settimeout(5)
s.connect((HOST, PORT))
banner = s.recv(1024)
print(f"SSH Server Banner bytes: {repr(banner)}")
s.close()

# Now connect with paramiko
client = paramiko.SSHClient()
client.set_missing_host_key_policy(paramiko.AutoAddPolicy())

try:
    print(f"Attempting SSH login as '{USER}'...")
    client.connect(
        HOST,
        port=PORT,
        username=USER,
        password=PASS,
        timeout=10,
        look_for_keys=False,
        allow_agent=False,
        disabled_algorithms=dict(pubkeys=['rsa-sha2-512', 'rsa-sha2-256', 'ssh-rsa'])
    )
    print("SSH Authentication SUCCEEDED!")

    # Test channel / exec
    stdin, stdout, stderr = client.exec_command("ps")
    time.sleep(1)
    out = stdout.read().decode('utf-8', errors='replace')
    print("Output of 'ps' over SSH:")
    print(out)

    client.close()
    print("SSH Test Complete: SUCCESS")
except Exception as e:
    print(f"SSH Test encountered: {type(e).__name__}: {e}")
    import traceback
    traceback.print_exc()
