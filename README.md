# EspressoOS — A `no_std` Unix-like Operating System in Rust for ESP32-S3

[![Rust Version](https://img.shields.io/badge/Rust-Xtensa%20(esp)-orange?logo=rust)](https://github.com/esp-rs/rust)
[![Target Platform](https://img.shields.io/badge/Platform-ESP32--S3--WROOM--1--N16R8-blue?logo=espressif)](https://www.espressif.com/en/products/socs/esp32-s3)
[![License](https://img.shields.io/badge/License-MIT%20or%20Apache--2.0-teal)](LICENSE)
[![Status](https://img.shields.io/badge/Status-Interactive%20shell%20%2B%20WiFi%20%2B%20SSH%20on%20hardware-brightgreen)](#current-status-running-on-hardware)

---

**EspressoOS** is a Unix-like operating system written entirely from scratch in `no_std` Rust for the **ESP32-S3-WROOM-1-N16R8** development board (Xtensa LX7 dual-core, 16 MB flash, 8 MB PSRAM).

It behaves *"like Linux, but for the ESP32-S3"*: preemptive multitasking with a hand-written Xtensa context switch, a Virtual File System (everything-is-a-file), kernel device drivers, a stable syscall ABI, ELF userland programs that execute from PSRAM, a WiFi + TCP/IP stack, an SSH-2.0 server, and one interactive shell reachable **both** over the serial console **and** over SSH — with runtime **Wi-Fi management from the shell**.

All command output and the shell present in **English**; the whole system identifies itself as **EspressoOS**.

---

## Table of Contents

- [What's New](#whats-new)
- [Current Status (Running on Hardware)](#current-status-running-on-hardware)
- [Quick Start (TL;DR)](#quick-start-tldr)
- [1. Environment Prerequisites](#1-environment-prerequisites)
- [2. Build & Flash — Step by Step](#2-build--flash--step-by-step)
- [3. Connecting to the Shell](#3-connecting-to-the-shell)
  - [3a. Serial console (UART0)](#3a-serial-console-uart0)
  - [3b. SSH over the network](#3b-ssh-over-the-network)
- [4. Connecting to Wi-Fi from the shell](#4-connecting-to-wi-fi-from-the-shell)
- [5. Command Reference](#5-command-reference)
- [6. The Shell — prompts, redirection, pipes, exit](#6-the-shell--prompts-redirection-pipes-exit)
- [Architecture & Kernel Subsystems](#architecture--kernel-subsystems)
- [Repository Structure](#repository-structure)
- [Memory Map & Partition Table](#memory-map--partition-table)
- [Development Roadmap](#development-roadmap)
- [License](#license)
- [Contact](#contact)
- [Support](#support)

---

## What's New

The most recent development cycle brought the system from "boots + SSH login" to a **fully interactive multitasking OS**:

- **Preemptive multitasking works on hardware.** The context switch was rewritten around the real esp-hal exception/interrupt model: the switch happens by overwriting the saved trap frame (`*save_frame`) in the vector epilogue on both the `syscall` trap and the SYSTIMER interrupt. `init`, `sh`, `net`, and a `heartbeat` task all run concurrently.
- **Userland ELF execution from PSRAM.** User programs are compiled to per-process fixed-address slots, deployed into `/bin`, and executed from PSRAM mapped onto the **instruction bus** via the MMU (Harvard-split `.text`/`.data`, ROM `Cache_Ibus_MMU_Set`). A blocking `wait()` with a syscall-restart flag closes the `spawn → wait → exit → reap` loop.
- **One unified shell, everywhere.** The **interactive console (UART0)** now runs the same full kernel shell as **SSH** — every built-in command works locally with argument parsing.
- **Runtime Wi-Fi management.** New `wifi`, `ip`, and `nmcli`-compatible commands **scan** for networks and **connect/disconnect** at runtime, without recompiling.
- **Clean session handling.** `exit`/`quit`/`logout` closes an SSH session with a proper channel-close handshake (client prints `Connection to <ip> closed.`).
- **English + EspressoOS branding** across all runtime output, prompts, banners, and the SSH server identity (`SSH-2.0-EspressoOS_0.1`).

---

## Current Status (Running on Hardware)

EspressoOS **boots and runs on a physical ESP32-S3**, is reachable over SSH, and drives an interactive shell over both the serial console and the network.

| Capability | Status |
| :--- | :--- |
| Compiles & links for `xtensa-esp32s3-none-elf` (`--release`) | ✅ |
| Boots: HAL init, kernel heap, VFS (`ramfs` on `/` + `/tmp`, `devfs` on `/dev`, `procfs` on `/proc`, `sysfs` on `/sys`) | ✅ |
| **8 MB PSRAM** mapped, added to the heap, and **executable** (instruction-bus MMU mapping) | ✅ |
| **Preemptive multitasking** — hand-written Xtensa windowed-register context switch | ✅ on hardware |
| **Userland**: ELF programs loaded from `/bin` and executed from PSRAM (`spawn`/`wait`/`exit`) | ✅ on hardware |
| **WiFi (STA) + DHCP + TCP/IP** (`esp-wifi` + `smoltcp`) | ✅ obtains an IP |
| **SSH-2.0 server** (curve25519-sha256 · ssh-ed25519 · chacha20-poly1305@openssh) | ✅ `ssh youareme@<ip>` |
| **Interactive shell** over UART **and** SSH (30+ commands, args, redirection, pipes) | ✅ on hardware |
| **Runtime Wi-Fi CLI** (`wifi scan` / `wifi connect` / `ip` / `nmcli` shim) | ✅ builds; validate scan on hardware |
| Persistent `EspFs` on `/` (survives reboot) | ⚠️ falls back to `ramfs` on device (open bug) |

> **Known open items:** `EspFs::mount` returns `IoError` on hardware, so `/` currently uses `ramfs` (files do **not** survive reboot yet); running `/bin/*` userland binaries *from the kernel shell* is not wired (their stdout would need routing); and `scan` while associated may need a disconnect→scan→reconnect fallback depending on `esp-wifi` behavior.

---

## Quick Start (TL;DR)

```bash
# 0. One-time toolchain install (see §1 for detail)
cargo install espup --locked
cargo install espflash@3.3.0 --locked      # 3.x — NOT 4.x
espup install
. $HOME/export-esp.ps1                       # PowerShell   (Linux/macOS: source $HOME/export-esp.sh)

# 1. Enter the repo
cd EspressoOS

# 2. Set your boot Wi-Fi credentials (git-ignored file)
cp kernel/src/wifi_credentials.rs.example kernel/src/wifi_credentials.rs
#   edit WIFI_SSID / WIFI_PASSWORD inside it

# 3. Generate the partition table
python tools/partition-gen/partition_gen.py

# 4. Build + flash + open the serial monitor (one command)
cargo run --release
```

Then, on the serial monitor you'll see the boot log, the board's IP, and the shell prompt `EspressoOS:~$`. From another terminal: `ssh youareme@<board-ip>`.

---

## 1. Environment Prerequisites

You need the Espressif Xtensa Rust toolchain, flashing tools, and Python 3.

### 1.1 Install the toolchain & flashing tools

```bash
cargo install espup --locked
cargo install espflash@3.3.0 --locked
espup install
```

> **Use espflash 3.x — not 4.x.** This project targets `esp-hal 0.23`, whose image format predates the ESP-IDF App Descriptor that espflash **4.x** requires. With 4.x the flashed image is rejected by the bootloader (`no bootable app`). Pin `espflash@3.3.0` until the project migrates to `esp-hal 1.0`.

### 1.2 Load the environment in your shell

**Windows (PowerShell):**
```powershell
Set-ExecutionPolicy RemoteSigned -Scope CurrentUser   # once, if scripts are blocked
. $HOME\export-esp.ps1
```
**Linux / macOS:**
```bash
source $HOME/export-esp.sh
```
Verify: `espflash --version` (should report a 3.x version).

### 1.3 Hardware

Connect the **ESP32-S3** to the host over USB. The console is on **UART0** (`esp-println` `uart` feature), exposed by the on-board USB-to-UART bridge (CH343 / CP2102 / CH340) as e.g. `COM5` (Windows) or `/dev/ttyUSB0` (Linux). A **2.4 GHz Wi-Fi** network must be in range (the ESP32-S3 radio is 2.4 GHz only).

> If your board exposes only the native USB-Serial-JTAG peripheral, switch the `esp-println` feature in [`kernel/Cargo.toml`](kernel/Cargo.toml) from `uart` to `jtag-serial`.

---

## 2. Build & Flash — Step by Step

From the repository root (`EspressoOS/`):

### Step 1 — Generate the partition table
```bash
python tools/partition-gen/partition_gen.py
```
Parses [`partitions.csv`](partitions.csv) into `partitions.bin`, validating alignment/boundaries (see [Memory Map](#memory-map--partition-table)).

### Step 2 — Set your boot Wi-Fi credentials
```bash
cp kernel/src/wifi_credentials.rs.example kernel/src/wifi_credentials.rs
```
```rust
// kernel/src/wifi_credentials.rs — GIT-IGNORED, never committed
pub const WIFI_SSID: &str = "your-2.4GHz-ssid";
pub const WIFI_PASSWORD: &str = "your-password";
```
This is the network the board joins **at boot**. You can switch networks later at runtime with the `wifi connect` command (§4). `wifi_credentials.rs` is in `.gitignore`, so your password never lands in version control.

### Step 3 — Build (release is mandatory)
```bash
cargo build --release
```
> A **release** build is required — PSRAM and `esp-wifi` do not work in debug. The build script also compiles the userland programs and embeds them into the kernel image.

### Step 4 — Flash and monitor
```bash
cargo run --release
```
This runs `espflash flash --monitor` (configured in [`.cargo/config.toml`](.cargo/config.toml)): it writes the image and opens the serial monitor. Pick the serial port when prompted.

### Expected serial output
```
[kernel] PSRAM added to heap: 7340032 bytes @ 0x3c1f0000 (1MB reserved for Userland @ 0x3c0f0000)
[psram-exec] reserved PSRAM mapped to the instruction bus @ 0x42800000 (16 pages)
[psram-exec] OK: code EXECUTED from PSRAM returned 42 (expected 42)

========================================
   EspressoOS   ·   kernel
   Live console. Starting subsystems.
   Kernel heap: 7471104 bytes
========================================
[kernel] userland: 10 binaries installed/updated in EspFs
[kernel] starting interactive console (kernel shell) on UART0...
[kernel] task 'shell' created (tid=1)
[kernel] task 'heartbeat' created (tid=2)
[kernel] task 'net' created (tid=3)
[net] connecting to SSID 'your-ssid'...
[net] associated with AP; negotiating DHCP...
[net] IP = 192.168.2.146
[net] SSH listening on port 22, ECHO on 2323, OTA on 3300

EspressoOS shell. Type 'help' to see the commands.
EspressoOS:~$
```
The on-board LED blinks (~1 Hz) as the heartbeat proof-of-multitasking. If it doesn't, adjust `LED_GPIO` in [`main.rs`](kernel/src/main.rs) (typically GPIO 2 or GPIO 48).

Monitor controls: **Ctrl+R** resets the chip, **Ctrl+C** exits the monitor.

---

## 3. Connecting to the Shell

EspressoOS runs **one** shell, reachable two ways. Both share the same command set and behavior.

### 3a. Serial console (UART0)

The serial monitor opened by `cargo run --release` **is** an interactive terminal. After boot you get the prompt:
```
EspressoOS:~$
```
Type commands directly. Examples:
```
EspressoOS:~$ help
EspressoOS:~$ ls /
EspressoOS:~$ cat /etc/passwd
EspressoOS:~$ wifi status
```

### 3b. SSH over the network

Once the board prints its IP and `SSH listening on port 22`, log in from any standard OpenSSH client on the same network:

```bash
ssh youareme@192.168.2.146          # then enter the dev password
```

| Item | Value | Where |
| :--- | :--- | :--- |
| Username | `youareme` | `DEV_USER` in [`ssh/config.rs`](kernel/src/drivers/ssh/config.rs) |
| Password | `851963Y@#` | `DEV_PASSWORD` in [`ssh/config.rs`](kernel/src/drivers/ssh/config.rs) |
| Host key | stable ed25519 (fixed dev seed) | `HOST_KEY_SEED` in `ssh/config.rs` |

The SSH prompt shows the logged-in user: `youareme@EspressoOS:~$`. The host key is derived from a fixed dev seed, so its fingerprint is **stable across reboots and re-flashes** — you won't need to clear `known_hosts`.

You can also check the raw network path with the TCP echo server on port 2323:
```bash
printf 'hello\n' | nc 192.168.2.146 2323     # echoes "hello" back
```

> **Security note (dev-only):** the SSH password and host-key seed are placeholders embedded in the binary for development. Change `DEV_USER` / `DEV_PASSWORD` / `HOST_KEY_SEED` for your setup, and do **not** expose this server to the internet.

---

## 4. Connecting to Wi-Fi from the shell

The board joins the network in `wifi_credentials.rs` **at boot**. To **scan** and **switch networks at runtime**, use the `wifi` command (or the `nmcli`-compatible shim). These work from either the console or SSH.

### The `wifi` command

```
EspressoOS:~$ wifi scan
Scanning for networks...
SSID                              RSSI   CH  SEC
Neighbor                           -48    6  WPA
Neighbor_5G                        -71   11  WPA
CafeLibre                          -80    1  open

EspressoOS:~$ wifi connect "Neighbor" "[PASSWORD]"
Connecting to 'Neighbor'...
(use 'wifi status' to check the result)

EspressoOS:~$ wifi status
state:  Connected
ssid:   Neighbor
ip:     192.168.2.146

EspressoOS:~$ ip
wlan0: 192.168.2.146  ssid "Neighbor"  state Connected

EspressoOS:~$ wifi disconnect
Disconnecting...
```

- **SSIDs with spaces** work because the shell honors quotes: `wifi connect "My Home Net" "pass"`.
- For an **open network**, omit the password: `wifi connect "GuestWifi"`.
- `wifi scan` briefly pauses (~1–2 s) while the radio scans; the SSID list is sorted by signal strength.

### The `nmcli` shim (familiar syntax)

An `nmcli`-compatible front-end maps the common operations to the same engine. A no-op `sudo` prefix is also accepted (EspressoOS has no privilege separation).

| `nmcli` command | Equivalent |
| :--- | :--- |
| `nmcli device status` | `wifi status` |
| `nmcli radio wifi on` | (no-op — the radio is always on) |
| `nmcli device wifi list` | `wifi scan` |
| `nmcli device wifi connect "SSID" password "PASS"` | `wifi connect "SSID" "PASS"` |
| `sudo nmcli device wifi connect "SSID" password "PASS"` | same (the `sudo` is ignored) |

```
EspressoOS:~$ nmcli device wifi list
EspressoOS:~$ sudo nmcli device wifi connect "Neighbor" "[PASSWORD]"
```

> **How it works:** the network task (`net`) is the sole owner of the Wi-Fi controller. The shell **enqueues** a command; the `net` task executes it in its service loop (scan is a blocking radio operation; `connect` reconfigures credentials, reconnects, and forces a fresh DHCP lease).

---

## 5. Command Reference

Type `help` in the shell to print this list. Multiple file arguments and quoted strings are supported everywhere.

### Shell built-ins

| Command | Syntax | Description |
| :--- | :--- | :--- |
| `help` | `help` | Show the command list. |
| `clear` | `clear` | Clear the screen (ANSI). |
| `echo` | `echo [-n] TEXT...` | Print text; `-n` suppresses the trailing newline. |
| `uptime` | `uptime` | Time since boot (days/h/m/s). |
| `free` | `free` | Kernel heap usage (total / used / free). |
| `ps` | `ps` | List scheduler tasks. |
| `reboot` | `reboot` | Software-reset the board. |
| `exit` | `exit` \| `quit` \| `logout` | End the SSH session; on the console, restart the shell. |

### Filesystem

| Command | Syntax | Description |
| :--- | :--- | :--- |
| `ls` | `ls [PATH]` | List a directory (defaults to CWD). Dirs are suffixed `/`, devices `@`. |
| `cd` | `cd [PATH]` | Change directory (defaults to `/`). |
| `pwd` | `pwd` | Print the current directory. |
| `cat` | `cat FILE...` | Print file contents. |
| `mkdir` | `mkdir DIR...` | Create directories. |
| `touch` | `touch FILE...` | Create empty files. |
| `rm` | `rm FILE...` | Remove files. |
| `write` | `write FILE TEXT...` | Write TEXT into FILE (truncates). |

```
EspressoOS:~$ mkdir /tmp/demo
EspressoOS:~$ write /tmp/demo/hello.txt hola mundo
EspressoOS:~$ cat /tmp/demo/hello.txt
hola mundo
EspressoOS:~$ ls /tmp/demo
hello.txt
```

### Networking

| Command | Syntax | Description |
| :--- | :--- | :--- |
| `wifi` | `wifi status \| scan \| connect "SSID" [PASS] \| disconnect` | Runtime Wi-Fi management (§4). |
| `ip` | `ip` | Show the `wlan0` address, SSID, and link state. |
| `nmcli` | `nmcli device status \| device wifi list \| device wifi connect "SSID" password "PASS"` | `nmcli`-compatible shim (§4). |
| `sudo` | `sudo COMMAND [ARGS...]` | Run a command (no privilege separation; the prefix is a no-op). |

### Hardware & buses

| Command | Syntax | Description |
| :--- | :--- | :--- |
| `i2c` | `i2c scan` · `i2c read ADDR_HEX LEN(1..64)` · `i2c write ADDR_HEX B0 [B1 ...]` | Master I2C on `/dev/i2c0`. |
| `spi` | `spi transfer B0 [B1 ...]` | Full-duplex SPI transfer on `/dev/spi0`. |
| `sha256` | `sha256 [TEXT]` | Hardware SHA-256 of TEXT. |
| `power` | `power sleep [SECONDS]` · `power deep-sleep [SECONDS]` | Light/Deep sleep (deep-sleep reboots on wake). |
| `ble` | `ble status` · `ble advertise` | Bluetooth LE status / start advertising as `EspressoOS`. |

```
EspressoOS:~$ i2c scan
EspressoOS:~$ i2c write 0x3c 0x00 0xAE
EspressoOS:~$ spi transfer 0x9F 0x00 0x00
EspressoOS:~$ sha256 hello
```

### System & advanced

| Command | Syntax | Description |
| :--- | :--- | :--- |
| `syscalltest` | `syscalltest` | Exercise the syscall ABI end to end. |
| `smp` | `smp` | Multicore (SMP) status. Requires the `smp` build feature to schedule on core 1. |
| `pms` | `pms [world1]` | Memory-protection (PMS) status; `pms world1` re-applies World-1 enforcement. Requires the `pms` build feature. |
| `ota` | `ota status` · `ota set factory\|ota0` · `ota rx` · `ota apply` | A/B firmware update: inspect `otadata`, select the boot slot, check the received buffer, and flash the inactive slot (image is received over TCP :3300). |

---

## 6. The Shell — prompts, redirection, pipes, exit

**Two prompts, one shell.** The prompt shows the current directory (`/` is displayed as `~`, like bash):
- **Serial console:** `EspressoOS:~$` (no user — there's no local login).
- **SSH:** `youareme@EspressoOS:~$` (the authenticated `DEV_USER`).

**Redirection & pipes:**
```
EspressoOS:~$ echo saved > /tmp/a.txt          # truncate
EspressoOS:~$ echo more >> /tmp/a.txt          # append
EspressoOS:~$ ls / | cat                        # pipe (first stage is executed)
```
> Multi-stage pipelines are parsed; currently the **first stage** runs (full N-stage piping in userland is a work item).

**Ending a session:** `exit` (also `quit` / `logout`).
- Over **SSH**, it performs a clean channel-close handshake — your client prints `Connection to <ip> closed.` and the server re-listens for the next connection.
- On the **console**, it prints `logout` and restarts the shell.

**Concurrency:** the console and an SSH session can be used at the same time; command output is routed to the correct session per command. (The current working directory is shared global state between them.)

---

## Architecture & Kernel Subsystems

- **Arch (Xtensa LX7)** — [`arch/xtensa`](kernel/src/arch/xtensa): hand-written windowed-register context switch ([`context.rs`](kernel/src/arch/xtensa/context.rs)), exception/interrupt vectors with a **preempt-in-epilogue** switch ([`interrupts.rs`](kernel/src/arch/xtensa/interrupts.rs), [`syscall/trap.rs`](kernel/src/syscall/trap.rs)), SYSTIMER ([`timer.rs`](kernel/src/arch/xtensa/timer.rs)), SMP-ready `Mutex`.
- **Memory** — [`mm`](kernel/src/mm): 8 MB octal PSRAM added to the `esp-alloc` heap ([`heap.rs`](kernel/src/mm/heap.rs)); the first 1 MB is reserved and **mapped to the instruction bus** so userland executes from PSRAM ([`psram_exec.rs`](kernel/src/mm/psram_exec.rs)); PMS memory protection behind `--features pms` ([`mpu.rs`](kernel/src/mm/mpu.rs)).
- **Scheduler & processes** — [`scheduler`](kernel/src/scheduler): round-robin over per-task frames; `spawn`/`exit`/`wait`, zombie reaping, blocking `wait` via a syscall-restart flag; SMP core-1 run-queue behind `--features smp`.
- **VFS** — [`vfs`](kernel/src/vfs): everything-is-a-file (`open`/`close`/`read`/`write`/`seek`/`readdir`), `devfs` (`/dev/console`, `/dev/null`, `/dev/zero`, `/dev/i2c0`, `/dev/spi0`), `ramfs`, `procfs` (`/proc`), `sysfs` (`/sys`), pipes.
- **Filesystem** — [`fs`](kernel/src/fs): `EspFs`, a pure-Rust log-structured, wear-leveled FS over internal NOR flash, mounted at `/` (currently falls back to `ramfs` on device).
- **Syscalls** — [`syscall`](kernel/src/syscall): stable ABI (`a2`=number, `a3..a8`=args); real `syscall`-instruction trap under `--features syscall-trap` (default).
- **Userland** — [`userland`](userland): `no_std` `libc` + ELF programs (`init`, `sh`, `cat`, `echo`, `ls`, `ota`, `ping`, `sntp`, `netstat`, `httpd`) built to per-process PSRAM slots by [`kernel/build.rs`](kernel/build.rs) and deployed to `/bin`.
- **Drivers** — [`drivers`](kernel/src/drivers): GPIO, UART, I2C, SPI, crypto/SHA, power, BLE, and Wi-Fi ([`wifi.rs`](kernel/src/drivers/wifi.rs)) bound to `smoltcp`.
- **SSH-2.0 server** — [`drivers/ssh`](kernel/src/drivers/ssh): curve25519 KEX, ed25519 host key, chacha20-poly1305 transport, password auth, session channel bridged to the shell.
- **OTA** — [`ota`](kernel/src/ota): A/B slots + `otadata`, TCP :3300 receiver buffered in PSRAM, `ota apply`.

---

## Repository Structure

```
EspressoOS/
├── .cargo/config.toml       # Xtensa target + `cargo run` = espflash flash --monitor
├── bootloader/              # 2nd-stage bootloader (stub crate, standalone)
├── kernel/                  # Kernel crate (package: espressoos-kernel, binary: kernel)
│   ├── build.rs             # Builds userland ELFs → per-slot linker scripts → embeds them
│   ├── src/
│   │   ├── arch/xtensa/     # context switch, vectors, timer, sync
│   │   ├── drivers/         # gpio, uart, i2c, spi, crypto, power, ble, wifi, ssh/
│   │   ├── fs/              # espfs, ramfs, procfs, sysfs, devfs
│   │   ├── mm/              # heap, psram_exec, mpu
│   │   ├── scheduler/       # tasks, policy, processes, core_sync (SMP)
│   │   ├── shell/           # commands/, parser, remote (console+SSH runner)
│   │   ├── syscall/         # table, handler, trap
│   │   ├── vfs/             # inode, file, devfs, pipe, socket, mount
│   │   ├── ota/             # A/B slots + otadata
│   │   ├── wifi_credentials.rs   # GIT-IGNORED boot Wi-Fi SSID/password
│   │   └── main.rs          # boot sequencer
│   └── Cargo.toml
├── userland/                # no_std libc + /bin programs (ELF, run from PSRAM)
│   ├── libc/                # syscall wrappers, _start
│   └── apps/src/bin/        # init, sh, cat, echo, ls, ota, ping, sntp, netstat, httpd
├── tools/                   # partition-gen (CSV→bin), test harnesses
├── partitions.csv           # 16 MB flash layout
└── README.md                # this file
```

---

## Memory Map & Partition Table

The **16 MB** external SPI flash is laid out for the A/B update scheme, honoring 4 KB erase sectors and 64 KB app alignment:

```
0x000000 ┤ Bootloader (2nd stage, 32 KB)
0x008000 ┤ Partition table (3072 B)
0x009000 ┤ NVS (24 KB)
0x00F000 ┤ otadata (A/B boot control, 8 KB)
0x020000 ┤ factory app — Slot A (primary kernel, 4 MB)
0x420000 ┤ ota_0 app  — Slot B (secondary kernel, 4 MB)
0x820000 ┤ filesystem (EspFs / ramfs, ~7.8 MB)
0xFF0000 ┤ coredump (64 KB)
```

Flash layout constants live in [`prelude.rs`](kernel/src/prelude.rs); the table is generated from [`partitions.csv`](partitions.csv) by `tools/partition-gen/partition_gen.py`.

**RAM:** 512 KB internal SRAM (kernel heap + esp-wifi `Internal` allocations) plus 8 MB PSRAM (general heap + the reserved 1 MB executable userland region at data alias `0x3c0f0000` / instruction alias `0x42800000`).

---

## Development Roadmap

Structured into 10 incremental phases. Bring-up (P0), memory/PSRAM (P1), **preemptive** multitasking (P2), the syscall ABI + **userland execution** (P6), and networking with an SSH server (P7) are **verified on hardware**. Bus drivers (P3) and persistent `EspFs` (P4) are wired and compile-clean (P4 persistence has an open device bug). OTA (P5), PMS (P8, `--features pms`), and SMP (P9, `--features smp`) are implemented behind opt-in features so the default image stays the known-good one.

| Phase | Title | Status |
| :--- | :--- | :--- |
| P0 | Bring-up (clock, heap, UART console, VFS, scheduler) | ✅ hardware |
| P1 | Memory management (8 MB PSRAM heap) | ✅ hardware |
| P2 | Task scheduler (preemptive context switch) | ✅ hardware |
| P3 | Bus drivers (I2C `/dev/i2c0`, SPI `/dev/spi0`) | ✅ wired |
| P4 | Storage & filesystems (`EspFs` on `/`) | ⚠️ ramfs fallback on device |
| P5 | OTA A/B updates (TCP :3300 → `ota apply`) | ✅ wired |
| P6 | Syscalls & userland (ABI + ELF exec from PSRAM) | ✅ hardware |
| P7 | Networking (WiFi STA) + SSH-2.0 server + shell | ✅ hardware |
| P8 | Memory protection (PMS, World-0/World-1) | 🔒 `--features pms` |
| P9 | SMP dual-core (APP_CPU run-queue) | 🔒 `--features smp` |

**Next steps:** fix `EspFs` persistence on device; wire launching `/bin/*` userland binaries from the kernel shell (with stdout routing); full N-stage pipelines; verify OTA boot-switch against the stock bootloader.

---

## License

Licensed under the **MIT License**. See [LICENSE](LICENSE) for details.

---

## Contact

Author: **Jorge Gaspar Beltre Rivera**  
Project: **EspressoOS - A `no_std` Unix-like Operating System in Rust for ESP32-S3**

 [![GitHub](https://img.shields.io/badge/GitHub-181717?style=for-the-badge&logo=github&logoColor=white)](https://github.com/JorgeGBeltre)
 [![LinkedIn](https://img.shields.io/badge/LinkedIn-0A66C2?style=for-the-badge&logo=linkedin&logoColor=white)](https://www.linkedin.com/in/jorge-gaspar-beltre-rivera/)
 [![Email](https://img.shields.io/badge/Email-EA4335?style=for-the-badge&logo=gmail&logoColor=white)](mailto:Jorgegaspar3021@gmail.com)

---

##  Support

This project is developed independently.

Even a small contribution helps me dedicate more time to development, testing, and releasing new features.

 [![Buy Me a Coffee](https://img.shields.io/badge/Buy_Me_a_Coffee-FFDD00?style=for-the-badge&logo=buy-me-a-coffee&logoColor=black)](https://www.paypal.com/donate/?hosted_button_id=2VLA8BWT967LU)
