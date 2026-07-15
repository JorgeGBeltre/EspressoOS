# EspressoOS — A `no_std` Unix-like Operating System in Rust for ESP32-S3

[![Rust Version](https://img.shields.io/badge/Rust-Xtensa%20(esp)-orange?logo=rust)](https://github.com/esp-rs/rust)
[![Target Platform](https://img.shields.io/badge/Platform-ESP32--S3--WROOM--1--N16R8-blue?logo=espressif)](https://www.espressif.com/en/products/socs/esp32-s3)
[![License](https://img.shields.io/badge/License-MIT)](LICENSE)
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

The most recent cycle made `/bin` actually runnable — programs, arguments and pipelines — and it took a detour through the linker to get there:

- **Userland programs run from the shell.** `/bin/echo hola | /bin/cat` works. Getting there needed load-time relocation, because **the LLVM Xtensa backend refuses to emit PIC at all** (`PIC relocations is not supported`, verified against `core` itself), so there is no PIE to load and a program linked for one address could only ever run at that address. Eight of the ten utilities shared a single fixed slot, which meant `ls` and `cat` could not run at the same time — pipelines were not unwired, they were impossible.
- **Relocation without PIC.** An ISA quirk pays for it: Xtensa cannot encode a 32-bit absolute in an instruction, so every far reference goes through the literal pool — and literals are *data*. Link with `ld --emit-relocs` and moving an image means patching a few dozen words: **11 for `cat`, 48 for `sh`**. All 879 `R_XTENSA_SLOT0_OP` relocations across the binaries are PC-relative and target their own `.text`, so a uniform bias leaves them correct and the loader skips them. No instruction is ever decoded. [`kernel/build.rs`](kernel/build.rs) digests the relocations at build time into a flat `u32` fixup table (44 B for `cat`) appended to the ELF, so the kernel parses no section headers.
- **A pool of 32 slots.** Any binary loads into any free slot, so two instances of the *same* program coexist — the thing PIE would have given us. Verified on hardware with overlapping timestamps.
- **argv, and an ABI bug it exposed.** `_start` is `#[naked]`, so it has no `entry` instruction — and on Xtensa the register window is rotated by the callee's `entry`, not by the call. It was running in the caller's window and reading a stale `a2`. Latent since the userland existed, invisible until something looked at that register.
- **`root:root` is gone.** `/etc/passwd` was seeded with two plaintext accounts and `auth.rs` consults it *before* the compiled credential, so changing `DEV_PASSWORD` did not close them — and once EspFs persisted, a stale copy survived re-flashing. The seed is removed; a file that exists now is deliberate and the kernel says so at boot.

The cycle before it made storage persist and rebuilt session I/O around file descriptors:

- **`EspFs` persists on hardware.** `EspFs::mount` used to fail with `IoError`. The cause was not in the filesystem: `esp-storage` derives the flash capacity from **byte 3 of the image header at flash offset `0x0000`**, and espflash writes `FlashSize::default()` (**4 MB**) there unless told otherwise — so every access above `0x400000`, which is both `fs` *and* `ota_0`, was rejected as out of bounds. Fixed with [`espflash.toml`](espflash.toml) (§2). Files now survive a power cycle.
- **The partition table on the chip is now yours.** Same root cause, second victim: without `partition_table` in `espflash.toml`, espflash flashes *its own* default table (`nvs` / `phy_init` / `factory@0x10000`), so `partitions.csv` had never actually reached the device. `otadata` was landing on top of `phy_init`.
- **One session, one process.** The serial console and each SSH session now each own a **pid, an fd table, and a `SessionChannel`**. `emit()` collapsed to `vfs::write(1, …)`, redirection became a plain `dup2` swap, and the global `Sink`/`OUTPUT`/`BASE`/`CWD` state and the whole single-session SSH bridge (`shell/remote.rs`, −260 lines) are gone. `ONLCR` (`\n` → `\r\n`) lives in the channel, where a terminal keeps it — so `echo x > f` finally writes plain LF.
- **The VFS no longer does I/O under a lock.** `vfs::read`/`write` used to hold the global fd-table `Mutex` — which **disables interrupts** — across `Inode::read_at`/`write_at`. They now snapshot the fd, release the guard, do the I/O unlocked, and re-credit the offset. That removed a class of hard wedge: a pipe read on an empty pipe parked the task while still holding the lock, and every other task then spun on it forever with interrupts off.
- **Preemptive multitasking works on hardware.** The context switch was rewritten around the real esp-hal exception/interrupt model: the switch happens by overwriting the saved trap frame (`*save_frame`) in the vector epilogue on both the `syscall` trap and the SYSTIMER interrupt. `shell`, `net` and `heartbeat` run concurrently, alongside the scheduler's idle task.
- **Userland ELF execution from PSRAM.** User programs are deployed into `/bin` and executed from PSRAM mapped onto the **instruction bus** via the MMU (Harvard-split `.text`/`.data`, ROM `Cache_Ibus_MMU_Set`). A blocking `wait()` with a syscall-restart flag closes the `spawn → wait → exit → reap` loop.
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
| Boots: HAL init, kernel heap, VFS (`EspFs` on `/`, `ramfs` on `/tmp`, `devfs` on `/dev`, `procfs` on `/proc`, `sysfs` on `/sys`) | ✅ |
| **8 MB PSRAM** mapped — 7 MB into the kernel heap, 1 MB reserved and executable on the instruction bus | ✅ |
| **Preemptive multitasking** — hand-written Xtensa windowed-register context switch | ✅ on hardware |
| **Userland**: `/bin` programs run from the shell, with argv (`/bin/echo hola mundo`) | ✅ on hardware |
| **Load-time relocation** — no PIC on this target, so images are patched into a free slot | ✅ on hardware |
| **32-slot pool** — two instances of the *same* binary run at once (overlapping timestamps) | ✅ on hardware |
| **WiFi (STA) + DHCP + TCP/IP** (`esp-wifi` + `smoltcp`) | ✅ obtains an IP |
| **SSH-2.0 server** (curve25519-sha256 · ssh-ed25519 · chacha20-poly1305@openssh) | ✅ `ssh youareme@<ip>` |
| **Interactive shell** over UART **and** SSH (~29 commands, args, redirection) | ✅ on hardware |
| **N-stage pipelines** (`/bin/ls / /tmp \| /bin/cat`) | ✅ on hardware |
| **Runtime Wi-Fi CLI** (`wifi scan` / `wifi connect` / `ip` / `nmcli` shim) | ✅ builds; validate scan on hardware |
| **Persistent `EspFs` on `/`** (survives a power cycle) | ✅ on hardware |
| **Per-session I/O**: console and SSH each own a pid, an fd table and a channel | ✅ on hardware |
| Your `partitions.csv` is the table on the chip (6 entries, kernel at `0x20000`) | ✅ on hardware |

> **Known open items:**
> - **A pipeline stage is always a `/bin` program, never a built-in.** A built-in runs inside the shell's own task, so it cannot run concurrently with the rest — and running the stages one after another is not a simplification but a deadlock, since the first would fill the 4 KB pipe with nobody draining it. There is no `fork` here to escape with. `wifi | cat` therefore fails with a clear error rather than doing something surprising.
> - **`ls`, `cat` and `echo` exist twice** — as built-ins and as `/bin` programs — and the two do not agree yet. `echo`, `cat` and `ls` in `/bin` honour argv; the other userland programs still ignore it.
> - **Userland cannot see its own working directory.** The kernel keeps a cwd per process, but there is no `getcwd` syscall and `vfs::mount::normalize` rejects any path not starting with `/`. So after `cd /tmp`, the built-in `ls` lists `/tmp` and `/bin/ls` lists `/`.
> - **No syscall validates a user pointer.** `sys_wait` writes the exit status wherever it is told; `user_slice` only checks for null. Harmless while every userland program is ours, but it is an arbitrary kernel write waiting for a program with a bug.
> - If an SSH client stops reading, the shell task spins yielding until it drains — a livelock, not a hang: the scheduler is round-robin, so the drain task still runs.
> - `scan` disconnects, scans and reconnects, so it drops any SSH session over Wi-Fi (§4).
> - OTA is built into the default image and has never been verified end to end against the stock bootloader.

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

# 3. Build + flash + open the serial monitor (one command)
#   espflash.toml is committed and supplies the flash size + partition table (see §2 Step 1)
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

### Step 1 — Check `espflash.toml` (do not skip this)

[`espflash.toml`](espflash.toml) is committed and needs no edits, but it is worth knowing what it does, because **both of its lines are load-bearing and neither fails loudly**:

```toml
partition_table = "partitions.csv"
[flash]
size = "16MB"
```

- **`size`** — espflash does *not* use the flash size it autodetects from the chip for the image header. Without this line it writes `FlashSize::default()` = **4 MB** into byte 3 of the header at `0x0000`, and `esp-storage` derives its capacity from exactly that byte. Everything above `0x400000` — the `fs` partition *and* `ota_0` — then fails with `OutOfBounds`, surfacing as `EspFs::mount` returning `IoError` on a build that compiles perfectly.
- **`partition_table`** — without it, espflash flashes **its own default table** (`nvs` / `phy_init` / `factory@0x10000`), not yours. The kernel reads `layout::*` from [`prelude.rs`](kernel/src/prelude.rs) directly and never consults the table, so the mismatch does not stop the boot — it corrupts. `layout::OTADATA_OFFSET` (`0xF000`) lands on top of espflash's `phy_init`.

Optionally, validate the CSV (this is what CI runs; espflash parses the CSV itself, so `partitions.bin` is not flashed):
```bash
python tools/partition-gen/partition_gen.py
```

### Step 2 — Set your boot Wi-Fi credentials
```bash
cp kernel/src/wifi_credentials.rs.example kernel/src/wifi_credentials.rs
```
```rust
// kernel/src/wifi_credentials.rs — git-ignored, never committed
pub const WIFI_SSID: &str = "your-2.4GHz-ssid";
pub const WIFI_PASSWORD: &str = "your-password";
```

These are a **fallback, not the boot network**. `wifi connect` (§4) saves the SSID and password to NVS at `0x9000`, and the boot path prefers a saved record over anything compiled in — so on a board that has connected even once, editing this file and re-flashing changes nothing. The boot log says which it used:

```
[net] using saved Wi-Fi credentials for 'Neighbor'      ← from flash; this file was ignored
[net] no saved Wi-Fi credentials; using compiled defaults
```

> `wifi_credentials.rs` really is git-ignored as of 2026-07-15. Until then this README promised it four times while `.gitignore` had no entry for it **and the file was tracked** — so following these instructions meant typing your password into something `git add .` would stage. Nothing leaked (the tracked copy held placeholders), but if you cloned this before that date, check your own history.

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
[kernel] flash: 16 MB usable
[kernel] / mounted on flash (espfs)
[kernel] userland: 11 binaries installed/updated in EspFs
[kernel] starting interactive console (kernel shell) on UART0...
[kernel] task 'shell' created (tid=1, pid=1)
[kernel] task 'heartbeat' created (tid=2)
[kernel] task 'net' created (tid=3)
[net] connecting to SSID 'your-ssid'...
[net] associated with AP; negotiating DHCP...
[net] IP = 192.168.2.146
[net] SSH listening on port 22, ECHO on 2323, OTA on 3300

EspressoOS shell. Type 'help' to see the commands.
EspressoOS:~$
```
Two of those lines are worth reading rather than skipping. **`flash: 16 MB usable`** is the kernel reporting the capacity `esp-storage` derived from the image header — if it says 4 MB, `espflash.toml` was not picked up and `EspFs` and OTA are both about to fail (the kernel prints an explicit warning naming the unreachable partitions). And **`/ mounted on flash (espfs)`** means the filesystem is real; `warning: EspFs::mount failed … using ramfs on /` means you have a working shell whose files vanish on reboot.

Also worth checking once: the ESP-IDF bootloader prints the partition table it actually read. It should list **six** entries with `factory` at `0x00020000`. If you see three (`nvs` / `phy_init` / `factory@0x10000`), that is espflash's built-in default table and `partitions.csv` never reached the chip.

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
EspressoOS:~$ cat /etc/rc
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

> **Security note (dev-only).** The credential and host-key seed are compiled into the binary. Change `DEV_USER` / `DEV_PASSWORD` / `HOST_KEY_SEED` in [`drivers/ssh/config.rs`](kernel/src/drivers/ssh/config.rs), and do **not** expose this server to the internet.
>
> Changing them is enough as of 2026-07-15, and was not before. `auth.rs` reads `/etc/passwd` **first** and returns success on a match there, and the kernel used to seed that file with `root:root` and `guest:guest` in plaintext — so `ssh root@<ip>` worked no matter what `DEV_PASSWORD` said. Worse, it was only written when absent, so once EspFs started persisting, a stale copy survived re-flashing.
>
> The seed is gone, so a fresh board has no such account. **A board flashed before that still does**: `rm /etc/passwd` on it, once. The kernel now warns at boot if the file exists, because it still overrides the compiled credential:
>
> ```
> [kernel] WARNING: /etc/passwd exists and overrides the compiled SSH credential
> ```
>
> `/etc/passwd` is still honoured if you create one deliberately — passwords in it are stored and compared in plaintext.

---

## 4. Connecting to Wi-Fi from the shell

At boot the board joins whichever network `wifi connect` last saved to NVS, falling back to the compiled `wifi_credentials.rs` only when no saved record exists (§2 Step 2). To **scan** and **switch networks at runtime**, use the `wifi` command (or the `nmcli`-compatible shim). These work from either the console or SSH.

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
- **`wifi scan` drops the network.** It disconnects the station, scans, then reconnects and re-acquires a DHCP lease — so **an SSH session over Wi-Fi will die on it**. Run it from the serial console. The list is sorted by signal strength. Duration is whatever the radio takes, not a fixed one or two seconds.

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
EspressoOS:~$ sudo nmcli device wifi connect "Neighbor" password "[PASSWORD]"
```

> **How it works:** the network task (`net`) is the sole owner of the Wi-Fi controller. The shell **enqueues** a command; the `net` task executes it in its service loop (scan is a blocking radio operation; `connect` reconfigures credentials, reconnects, forces a fresh DHCP lease, **and saves the SSID and password to NVS at `0x9000`** -- which makes that network the boot network from then on, surviving a re-flash of the kernel).

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
| `free` | `free` | Kernel heap usage, and PSRAM exec slots in use (userland images are not on the heap). |
| `ps` | `ps` | Show the **current** task's TID. It does not enumerate the task table. |
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
| `pms` | `pms [world1]` | Memory-protection (PMS) status. `pms world1` applies an experimental W^X policy (DRAM R+W, IRAM R+X), which **loosens** the no-access default set at boot rather than tightening it. Requires the `pms` build feature. |
| `ota` | `ota status` · `ota set factory\|ota0` · `ota rx` · `ota apply` | A/B firmware update: inspect `otadata`, select the boot slot, check the received buffer, and flash the inactive slot (image is received over TCP :3300). |

### Userland programs (`/bin`)

Anything the table above does not name is looked up in `/bin`, so `/bin/echo hola` runs the program while `echo hola` runs the built-in. The programs are ELF images relocated into a PSRAM slot at launch and reaped when they exit.

| Program | State |
| :--- | :--- |
| `/bin/echo [-n] TEXT...` | ✅ honours argv |
| `/bin/cat [FILE...]` | ✅ honours argv; no arguments reads **stdin**, which is what makes it useful in a pipeline |
| `/bin/ls [PATH...]` | ✅ honours argv; no arguments lists `/` (a program cannot see its cwd — see Next Steps) |
| `/bin/sleep` | ✅ holds a slot for 3 s; exists to test that two images coexist |
| `/bin/init`, `/bin/sh`, `/bin/ota`, `/bin/ping`, `/bin/sntp`, `/bin/netstat`, `/bin/httpd` | ⚠️ run, but ignore their arguments |

```
EspressoOS:~$ /bin/echo hola mundo
hola mundo
EspressoOS:~$ /bin/ls /bin | /bin/cat
cat
echo
init
...
EspressoOS:~$ free
            total         used         free
heap      7471104       171312      7299792
slots          32            0           32
```

`free` reports the slot pool as well as the heap, because a userland image is not on the heap — it lives in the reserved PSRAM region, and a leak there would otherwise stay invisible until the 33rd launch failed.

---

## 6. The Shell — prompts, redirection, pipes, exit

**Two prompts, one shell.** The prompt shows the current directory (`/` is displayed as `~`, like bash):
- **Serial console:** `EspressoOS:~$` (no user — there's no local login).
- **SSH:** `youareme@EspressoOS:~$` (the authenticated `DEV_USER`).

**Redirection:**
```
EspressoOS:~$ echo saved > /tmp/a.txt          # truncate
EspressoOS:~$ echo more >> /tmp/a.txt          # append
```
> Redirection is a `dup2` swap onto fd 1, so a command never learns it moved. `stderr` is fd 2 and `>` does not capture it. Files get plain **LF**: the `\n` → `\r\n` translation belongs to the terminal channel, not to `echo`.

**Pipes:**
```
EspressoOS:~$ /bin/echo hola mundo | /bin/cat
hola mundo
EspressoOS:~$ /bin/ls / /tmp | /bin/cat
/:
bin
etc
prueba.txt
/tmp:
```
Every stage is launched at once and the shell waits for them all — `ls` writes while `cat` reads. Running them one after another would deadlock the moment a stage's output exceeded the 4 KB pipe, since nothing would be draining it.

> **A pipeline stage must be a `/bin` program.** Built-ins run inside the shell's own task, so they cannot run concurrently with the rest, and there is no `fork` to escape with. `wifi | cat` fails before anything is allocated:
> ```
> EspressoOS:~$ wifi | /bin/cat
> shell: wifi: not found in /bin (a pipeline stage cannot be a built-in)
> ```
> This is why the examples spell out `/bin/`. `echo`, `cat` and `ls` exist both as built-ins and as programs; a bare `echo hola | cat` picks the programs, and for those three that is what you want. A stage's own `>` beats the pipe, as in a real shell: `/bin/ls > f | /bin/cat` sends `ls` to the file and leaves `cat` an empty pipe.

**Ending a session:** `exit` (also `quit` / `logout`).
- Over **SSH**, the shell task exits, the server sends a clean `CHANNEL_EOF`/`CHANNEL_CLOSE` — your client prints `Connection to <ip> closed.` — and the process is reaped.
- On the **console**, it prints `logout` and starts a fresh session (the banner reprints). It deliberately does *not* end the task: the serial port is the board's only local way in.

**Concurrency:** the console and an SSH session are two independent processes. Each has its own pid, its own fd table with `0/1/2` bound to its own `SessionChannel`, and its own working directory — `cd` in one does not move the other, and a new SSH session always starts at `/`. Children inherit all of it through `clone_fd_table`, which is what will make `exec` work.

---

## Architecture & Kernel Subsystems

- **Arch (Xtensa LX7)** — [`arch/xtensa`](kernel/src/arch/xtensa): hand-written windowed-register context switch ([`context.rs`](kernel/src/arch/xtensa/context.rs)), exception/interrupt vectors with a **preempt-in-epilogue** switch ([`interrupts.rs`](kernel/src/arch/xtensa/interrupts.rs), [`syscall/trap.rs`](kernel/src/syscall/trap.rs)), SYSTIMER ([`timer.rs`](kernel/src/arch/xtensa/timer.rs)), SMP-ready `Mutex`.
- **Memory** — [`mm`](kernel/src/mm): 8 MB octal PSRAM added to the `esp-alloc` heap ([`heap.rs`](kernel/src/mm/heap.rs)); the first 1 MB is reserved and **mapped to the instruction bus** so userland executes from PSRAM ([`psram_exec.rs`](kernel/src/mm/psram_exec.rs)); PMS memory protection behind `--features pms` ([`mpu.rs`](kernel/src/mm/mpu.rs)).
- **Scheduler & processes** — [`scheduler`](kernel/src/scheduler): round-robin over per-task frames (FIFO — `Task::priority` is carried but not yet consulted); `spawn`/`exit`/`wait`, zombie reaping, blocking `wait` via a syscall-restart flag; per-process cwd; `spawn_blocked` for tasks that must be set up before they can run; `reap_orphans` for processes no one can `wait()` for; SMP core-1 run-queue behind `--features smp`.
- **VFS** — [`vfs`](kernel/src/vfs): everything-is-a-file (`open`/`close`/`read`/`write`/`seek`/`readdir`/`dup`/`dup2`), per-process fd tables, `devfs` (`/dev/console`, `/dev/null`, `/dev/zero`, `/dev/i2c0`, `/dev/spi0`), `ramfs`, `procfs` (`/proc`), `sysfs` (`/sys`), pipes. `read`/`write` snapshot the fd, **release the fd-table lock, then do the I/O** — the lock disables interrupts, so a blocking inode underneath it would wedge the kernel.
- **Sessions** — [`session.rs`](kernel/src/session.rs): a `SessionChannel` (`Uart` or `Ssh`) plus `SessionConsole`, the inode that makes one look like a file. Owns `ONLCR`, non-blocking short writes (`WouldBlock` = retry, `IoError` = the session is gone), and the per-session rings the SSH server drains.
- **Filesystem** — [`fs`](kernel/src/fs): `EspFs`, a pure-Rust append-only record log over internal NOR flash, mounted at `/` and persistent across power cycles. It ping-pongs between two fixed halves on compaction and picks the live one from an alternating superblock, which spreads erases across those two halves -- but there is no erase counting anywhere, so do not read that as wear levelling.
- **Syscalls** — [`syscall`](kernel/src/syscall): stable ABI (`a2`=number, `a3..a8`=args); real `syscall`-instruction trap under `--features syscall-trap` (default).
- **Userland** — [`userland`](userland): `no_std` `libc` + ELF programs (`init`, `sh`, `cat`, `echo`, `ls`, `ota`, `ping`, `sntp`, `netstat`, `httpd`, `sleep`) built by [`kernel/build.rs`](kernel/build.rs), embedded in the kernel image and deployed to `/bin` on first boot. All link at one canonical address and the loader relocates them into whatever slot is free.
- **Relocation** — [`fs/elf.rs`](kernel/src/fs/elf.rs) + [`build.rs`](kernel/build.rs): the LLVM Xtensa backend rejects PIC, so there is no PIE and a fixed-address image would pin each program to one slot forever. Instead the host links with `ld --emit-relocs`, digests the result into a `u32`-per-fixup table appended to the ELF, and the kernel adds a bias to a few dozen literal words at load time. Only `R_XTENSA_32` needs patching — every `R_XTENSA_SLOT0_OP` is PC-relative within `.text` and survives a uniform bias untouched — so no instruction is decoded. Two subtleties the code carries in comments: `ld` leaves the addend at 0 for section symbols and the resolved value in the word, so the loader read-modify-writes rather than computing `S + A`; and text is written through the PSRAM **data** alias and executed through the **instruction** alias, so `Cache_WriteBack_All` + `Cache_Invalidate_ICache_All` sit between the two.
- **Slot pool** — [`mm/psram_exec.rs`](kernel/src/mm/psram_exec.rs): the reserved megabyte is 32 slots of 16 KB (text) paired with 32 of data. A slot is claimed with a CAS on a `u32` bitmap — not a `Mutex`, which here would disable interrupts to flip a bit — and rides on the `Process` so that reaping it returns the slot. `slot_text_exec` and `slot_text_write` are separate functions on purpose: they are two addresses for one piece of PSRAM, and writing through the wrong one does not fault, it just leaves the CPU fetching stale bytes.
- **Drivers** — [`drivers`](kernel/src/drivers): GPIO, UART, I2C, SPI, crypto/SHA, power, BLE, and Wi-Fi ([`wifi.rs`](kernel/src/drivers/wifi.rs)) bound to `smoltcp`.
- **SSH-2.0 server** — [`drivers/ssh`](kernel/src/drivers/ssh): curve25519 KEX, ed25519 host key, chacha20-poly1305 transport, password auth. One session at a time (one socket, one `Connection`, one channel). A `shell` request builds a `SessionShell`: create channel → `spawn_blocked` → `register_process` → `seed_fd_table` → `unblock_task`. The session ends when the shell process does.
- **OTA** — [`ota`](kernel/src/ota): A/B slots + `otadata`, TCP :3300 receiver buffered on the heap (which is mostly PSRAM, but nothing asks for that), `ota apply`.

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
│   │   ├── shell/           # commands/, parser  (one run_session for console+SSH)
│   │   ├── session.rs       # SessionChannel (Uart|Ssh) + SessionConsole inode
│   │   ├── syscall/         # table, handler, trap
│   │   ├── vfs/             # inode, file, devfs, pipe, socket, mount
│   │   ├── ota/             # A/B slots + otadata
│   │   ├── wifi_credentials.rs   # GIT-IGNORED boot Wi-Fi SSID/password
│   │   └── main.rs          # boot sequencer
│   └── Cargo.toml
├── userland/                # no_std libc + /bin programs (ELF, run from PSRAM)
│   ├── libc/                # syscall wrappers, _start
│   └── apps/src/bin/        # init, sh, cat, echo, ls, ota, ping, sntp, netstat, httpd, sleep
├── tools/                   # partition-gen (CSV→bin validator), test harnesses
├── espflash.toml            # flash size + partition table — both load-bearing (§2)
├── partitions.csv           # 16 MB flash layout (espflash reads this directly)
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
0x820000 ┤ filesystem (EspFs, ~7.8 MB)
0xFF0000 ┤ coredump (64 KB)
```

Flash layout constants live in [`prelude.rs`](kernel/src/prelude.rs) and the table comes from [`partitions.csv`](partitions.csv), which espflash flashes because [`espflash.toml`](espflash.toml) points it there.

> The two are **not** cross-checked at build time, and the kernel never reads the partition table — it addresses flash straight through `layout::*`. So a `prelude.rs` that disagrees with `partitions.csv` compiles, boots, and silently writes into the wrong partition. Keep them in sync by hand, and if you change either, re-read the bootloader's table dump on the next boot.

**RAM:** 512 KB internal SRAM (kernel heap + esp-wifi `Internal` allocations) plus 8 MB PSRAM (general heap + the reserved 1 MB executable userland region at data alias `0x3c0f0000` / instruction alias `0x42800000`).

---

## Development Roadmap

Structured into 10 incremental phases. Bring-up (P0), memory/PSRAM (P1), **preemptive** multitasking (P2), persistent storage (P4), the syscall ABI + **userland execution** (P6), and networking with an SSH server (P7) are **verified on hardware**. Bus drivers (P3) are wired and compile-clean. OTA (P5) is built into the default image -- it is not behind a feature, and its TCP receiver listens on :3300 on every boot -- but it has never been verified end to end. PMS (P8, `--features pms`) and SMP (P9, `--features smp`) are behind opt-in features so the default image stays the known-good one.

| Phase | Title | Status |
| :--- | :--- | :--- |
| P0 | Bring-up (clock, heap, UART console, VFS, scheduler) | ✅ hardware |
| P1 | Memory management (8 MB PSRAM heap) | ✅ hardware |
| P2 | Task scheduler (preemptive context switch) | ✅ hardware |
| P3 | Bus drivers (I2C `/dev/i2c0`, SPI `/dev/spi0`) | ✅ wired |
| P4 | Storage & filesystems (`EspFs` on `/`) | ✅ hardware — persists across power cycles |
| P5 | OTA A/B updates (TCP :3300 → `ota apply`) | ✅ wired |
| P6 | Syscalls & userland (ABI + ELF exec from PSRAM + argv + pipelines) | ✅ hardware |
| P7 | Networking (WiFi STA) + SSH-2.0 server + shell | ✅ hardware |
| P8 | Memory protection (PMS, World-0/World-1) | 🔒 `--features pms` |
| P9 | SMP dual-core (APP_CPU run-queue) | 🔒 `--features smp` |

**Next steps**, roughly in order of how soon each one bites:

1. **Validate user pointers in syscalls.** `sys_wait` writes the exit status to whatever address it is handed, and `user_slice` only rejects null. That was academic while nothing ran in userland; now that programs take arguments and read stdin, it is an arbitrary kernel write one bug away. This should land before the userland grows.
2. **Teach the rest of the userland argv.** `echo`, `cat` and `ls` honour it; `ota`, `ping`, `sntp`, `netstat`, `httpd`, `init` and `sh` take `(_argc, _argv)` and ignore them. Until then a built-in and its `/bin` twin can disagree.
3. **Give userland its working directory** — either a `getcwd` syscall, or have `vfs::mount::resolve` resolve relative paths against the calling process's cwd, which is what POSIX does and would make the shell's own `resolve()` redundant. Today the kernel tracks a cwd that no program can read.
4. **Run [`/etc/rc`](kernel/src/main.rs).** It is written on every boot and never executed — a userland init nearly for free, now that `exec` exists.
5. **Verify OTA end to end** — `ota_0` and `otadata` are reachable and the table on the chip is finally the right one, so this can be tested for the first time.
6. **Own bootloader** → Multiboot 2, to drop the ESP-IDF second stage.

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

## Support

This project is developed independently.

Even a small contribution helps me dedicate more time to development, testing, and releasing new features.

 [![Buy Me a Coffee](https://img.shields.io/badge/Buy_Me_a_Coffee-FFDD00?style=for-the-badge&logo=buy-me-a-coffee&logoColor=black)](https://www.paypal.com/donate/?hosted_button_id=2VLA8BWT967LU)
