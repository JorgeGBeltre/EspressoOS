# EspressoOS — A `no_std` Unix-like Operating System in Rust for ESP32-S3

[![Rust Version](https://img.shields.io/badge/Rust-Xtensa%20(esp)-orange?logo=rust)](https://github.com/esp-rs/rust)
[![Target Platform](https://img.shields.io/badge/Platform-ESP32--S3--WROOM--1--N16R8-blue?logo=espressif)](https://www.espressif.com/en/products/socs/esp32-s3)
[![License](https://img.shields.io/badge/License-MIT)](LICENSE)
[![Status](https://img.shields.io/badge/Status-Interactive%20shell%20%2B%20WiFi%20%2B%20SSH%20on%20hardware-brightgreen)](#2-status--running-on-hardware)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/JorgeGBeltre/EspressoOS)


---

**EspressoOS** is a Unix-like operating system written from scratch in `no_std` Rust for the **ESP32-S3-WROOM-1-N16R8** development board (Xtensa LX7 dual-core, 16 MB external flash, and 8 MB Octal PSRAM).

It operates under the Unix philosophy ("everything is a file"): preemptive multitasking with a handwritten Xtensa context switch, a Virtual File System (VFS) with `/dev`, `/proc`, and `/sys` mount points, kernel device drivers reached through a unified `ioctl` pattern, a frozen 30-call syscall ABI, **ELF userland binaries executing from PSRAM** (relocated at load time to bypass LLVM's lack of PIC support for Xtensa), a Wi-Fi and TCP/IP stack (`esp-wifi` + `smoltcp`), an **SSH-2.0 server**, a BLE advertiser, and interactive shells accessible over both serial console and SSH.

All runtime output and shells are in **English**, identifying the system as **EspressoOS** (SSH ident `SSH-2.0-EspressoOS_0.1`, BLE advertising name `EspressoOS`).

---

## Table of Contents

- [1. Hardware Target](#1-hardware-target)
- [2. Status — Running on Hardware](#2-status--running-on-hardware)
- [3. Architecture by Subsystem](#3-architecture-by-subsystem)
  - [3.1 Boot Sequence](#31-boot-sequence)
  - [3.2 Memory Architecture (Heap, PSRAM Exec Pool, Stacks)](#32-memory-architecture-heap-psram-exec-pool-stacks)
  - [3.3 Scheduler, Processes & Signals](#33-scheduler-processes--signals)
  - [3.4 Syscall ABI (Full Table)](#34-syscall-abi-full-table)
  - [3.5 Virtual File System (VFS), /dev, /proc, /sys, Pipes & Sockets](#35-virtual-file-system-vfs-dev-proc-sys-pipes--sockets)
  - [3.6 Device Drivers & ioctl Bus Pattern](#36-device-drivers--ioctl-bus-pattern)
  - [3.7 Authentication System & passwd Command](#37-authentication-system--passwd-command)
  - [3.8 Userland, ELF Loader & /bin/sh Shell](#38-userland-elf-loader--binsh-shell)
- [4. Build & Flash Guide](#4-build--flash-guide)
- [5. Complete Command Reference](#5-complete-command-reference)
- [6. The Shells — Prompts, Redirection, Pipes & Sessions](#6-the-shells--prompts-redirection-pipes--sessions)
- [7. Repository Structure](#7-repository-structure)
- [8. Memory Map & Partition Table](#8-memory-map--partition-table)
- [9. Known Issues & Technical Debt](#9-known-issues--technical-debt)
- [License](#license)
- [Contact](#contact)
- [Support](#support)
---

## 1. Hardware Target

| Item | Value |
| :--- | :--- |
| Board | ESP32-S3-WROOM-1-N16R8 development board |
| CPU | Xtensa LX7 dual-core (ProCpu + AppCpu), max frequency `CpuClock::max()` |
| Flash | 16 MB external SPI NOR |
| PSRAM | 8 MB Octal (`octal-psram`) — requires a release build (`--release`) |
| Console | UART0 via on-board USB-to-UART bridge (CH343 chip, USB `1a86:55d3`) |
| Radio | 2.4 GHz Wi-Fi (STA mode) + Bluetooth Low Energy (BLE) via `esp-wifi` |
| Toolchain | Espressif `esp` Rust toolchain (`xtensa-esp32s3-none-elf`), `esp-hal 0.23.1`, `espflash 3.3.0` |

> Note on serial port: The console is configured on UART0 (`uart` feature in `esp-println`). If your board only exposes native USB-Serial-JTAG, switch the feature in `kernel/Cargo.toml` from `uart` to `jtag-serial`.

---

## 2. Status — Running on Hardware

EspressoOS boots and executes on physical ESP32-S3 hardware. The system acquires an IP address via DHCP over Wi-Fi, accepts SSH connections, and drives interactive shells over serial and network connections.

### Hardware-Verified Capabilities

- **Compilation & Linking**: Clean compilation in `--release` mode for `xtensa-esp32s3-none-elf`.
- **Kernel Boot**: HAL initialization, 128 KB SRAM internal heap, VFS mounts (`/` EspFs on flash, `/tmp` ramfs in RAM, `/dev` devfs, `/proc` procfs, `/sys` sysfs).
- **PSRAM Execution (PSRAM-Exec)**: 8 MB PSRAM carve-up with 1 MB reserved for code execution on the instruction bus (`0x42800000`). Boot selftest verifies instruction execution in PSRAM returning code 42.
- **Preemptive Multitasking**: Scheduler driven by SYSTIMER at 100 Hz, 50 ms quantum, and Model B in-ISR context switching.
- **Dynamic Userland ELFs**: ELF binary execution from PSRAM with `argv` support, 32-slot pool, and load-time relocation without requiring Position Independent Code (PIC).
- **Wi-Fi + TCP/IP Stack**: Station (STA) mode, DHCP client, and active TCP/IP stack (`smoltcp`). Runtime Wi-Fi scan, connect, and disconnect. Credential persistence in flash NVS.
- **SSH-2.0 Server**: Server supporting curve25519-sha256, ssh-ed25519, and chacha20-poly1305@openssh.com. Password authentication reads `/etc/passwd`.
- **`passwd` Command & Credential Management**: Userland application `/bin/passwd` creates or updates `/etc/passwd` in persistent flash storage (EspFs).
- **Persistent EspFs Filesystem**: Log-structured filesystem on NOR flash surviving reboots and re-flashes.
- **Hardware Cryptography**: Driver `/dev/sha0` exposing the hardware SHA-256 accelerator.
- **Power Management & Reboot**: Driver `/dev/power` with working software reboot.
- **BLE Advertising**: Beacon advertisement as `EspressoOS` via `/bin/ble`.

> **Verified execution example on hardware (`ping` ICMP):**
> ```text
> EspressoOS:~$ ping 192.168.2.1
> PING 192.168.2.1 (ICMP Echo Request)...
> 64 bytes from 192.168.2.1: icmp_seq=0 time=21 ms
> 64 bytes from 192.168.2.1: icmp_seq=1 time=18 ms
> 64 bytes from 192.168.2.1: icmp_seq=2 time=4 ms
> 64 bytes from 192.168.2.1: icmp_seq=3 time=57 ms
> 
> --- 192.168.2.1 ping statistics ---
> 4 packets transmitted, 4 received, 0% packet loss
> ```

### Partial or Feature-Gated Capabilities

- **I2C (`/dev/i2c0`) & SPI (`/dev/spi0`)**: Master drivers and `ioctl` interface implemented and verified on open bus.
- **Light Sleep (`power sleep`)**: Suspended due to platform limitations; reboot and deep sleep are the recommended paths.
- **PMS Memory Protection / Dual-Core SMP**: Features `pms` and `smp` are fully implemented in code but disabled by default for simplicity and stability.
- **littlefs**: Static stub in `kernel/src/fs/littlefs/mod.rs` returning an empty read-only root. Not mounted by default.

---

## 3. Architecture by Subsystem

Module tree declared in `kernel/src/main.rs`: `arch`, `drivers`, `fs`, `mm`, `ota`, `prelude`, `scheduler`, `session`, `shell`, `syscall`, `vfs`, `wifi_credentials`.

### 3.1 Boot Sequence

Entry point located in `kernel/src/main.rs` (`#[esp_hal::main] fn main() -> !`):

1. **HAL Init**: `esp_hal::init` at max clock frequency (`CpuClock::max()`) and 8 MB Octal PSRAM initialization.
2. **PSRAM Carve-up**: 1 MB reserved at PSRAM base for userland executable slot pool (`psram_exec::set_data_base`). Remaining ~7 MB added to kernel heap (`mm::heap::add_psram`).
3. **PSRAM-Exec Verification**: Instruction mapping (`map_instruction`) and `selftest()` execution, copying a `movi a2, 42 / ret` template into a slot, syncing caches, and asserting return value 42.
4. **Core Driver Init**: Power management (`power`), UART (`uart`), interrupts (`arch::xtensa::interrupts::init`).
5. **VFS Mounts**:
   - Mount `/dev` (devfs).
   - Mount root `/` from NOR flash (`EspFs::mount()`). Falls back to `ramfs` if EspFs is unformatted or corrupt.
   - Mount `/tmp` (ramfs), `/proc` (procfs), `/sys` (sysfs).
6. **PID 0 File Descriptor Table**: Seed `/dev/console` for standard streams (fd 0, 1, 2).
7. **Userland Deployment**: `install_userland()` inspects the 35 embedded binaries and deploys or updates `/bin/*` on EspFs.
8. **System Files Setup**: `init_etc_files()` writes `/etc/rc`. If `/etc/passwd` exists, the kernel logs a warning noting custom SSH credentials are active.
9. **I2C and SPI Init**: Initialize `I2C0` and `SPI2`.
10. **Scheduler & Supervisor Bring-up**: Task creation:
    - `idle` (TID 0): System idle task.
    - `init-sup`: Supervisor task spawning `/bin/init` (which runs `/bin/sh`).
    - `heartbeat`: Blinks GPIO2 LED at 1 Hz as visual multitasking proof.
    - `net`: Network management task (`smoltcp` & Wi-Fi) with 24 KB stack.
11. **SYSTIMER & Scheduler Launch**: SYSTIMER started at 100 Hz, transferring execution to `scheduler::run()`.

---

### 3.2 Memory Architecture (Heap, PSRAM Exec Pool, Stacks)

Memory layout defined in `kernel/src/prelude.rs` (`layout`).

- **Kernel Heap** (`kernel/src/mm/heap.rs`): Uses `esp_alloc::HEAP` combining 128 KB internal SRAM and ~7 MB external PSRAM. Heap stats are exposed via `/bin/free` and `/proc/meminfo`.
- **PSRAM Executable Slot Pool** (`kernel/src/mm/psram_exec.rs`): Reserved 1 MB region split into:
  - 512 KB text region (mapped on instruction bus at `0x42800000`).
  - 512 KB data region (addressed at `0x3c170000`).
  - Asigned as 32 independent slots of 16 KB each. Managed via lock-free atomic bitmasks (`AtomicU32`).
  - Code is written through data alias and executed through instruction bus alias, maintaining cache coherence via `Cache_WriteBack_All` and `Cache_Invalidate_ICache_All`.
- **Stack Watermarking** (`kernel/src/scheduler/task.rs`): Task stacks are painted with `0xDEADBEEF` at creation. `stack_high_water()` monitors peak stack usage.
  - General task stack size: 16 KB.
  - Network task (`net`) stack size: 24 KB.
  - Standard safety rule requires at least a 25% free stack margin.

---

### 3.3 Scheduler, Processes & Signals

- **Multitasking Scheduler** (`kernel/src/scheduler/mod.rs`):
  - Strategy: Round-Robin selection among `Ready` tasks respecting core affinity.
  - Preemption: SYSTIMER generates 100 Hz ticks (10 ms). Quantum is set to 5 ticks (50 ms).
  - Context Switch (Model B): ISR frame dispatcher modifies the saved register frame (`save_frame`) in-place, swapping current and next task frame pointers.
- **Process Management** (`kernel/src/scheduler/process.rs`):
  - `Process` structure holds `pid`, `parent_pid`, `main_task`, state, working directory `cwd`, file descriptor table (`FdTable`), signal handlers, and assigned PSRAM slot.
  - Reaping releases file descriptors and returns the PSRAM slot to the pool. Orphaned processes are swept by `reap_orphans()`.
- **Signal Handling**:
  - Internal signal support for `SIGINT` (2), `SIGKILL` (9), and `SIGTERM` (15).
  - User/kernel guard: Pending signals are dispatched in `check_signals` before returning to user mode, protecting kernel routines from corruption.

---

### 3.4 Syscall ABI (Full Table)

The system call ABI is frozen across syscall numbers 0 to 29 (`kernel/src/syscall/table.rs`). Arguments are passed in `a3..a8`, syscall number in `a2`, and return value in `a2`.

| Number | Syscall | Description |
| :--- | :--- | :--- |
| 0 | `Read` | Read from file descriptor |
| 1 | `Write` | Write to file descriptor |
| 2 | `Open` | Open or create file |
| 3 | `Close` | Close file descriptor |
| 4 | `Ioctl` | Device input/output control (`/dev/*`) |
| 5 | `Exit` | Terminate current process |
| 6 | `Spawn` | Load and execute ELF binary in slot |
| 7 | `Wait` | Wait for child process termination |
| 8 | `Seek` | Reposition file offset |
| 9 | `Mkdir` | Create directory |
| 10 | `Unlink` | Remove file |
| 11 | `Readdir` | Read directory entry |
| 12 | `UptimeMs` | System uptime in milliseconds |
| 13 | `Sbrk` | Query available free memory |
| 14 | `Yield` | Yield CPU execution |
| 15 | `Signal` | Register signal handler (sigaction) |
| 16 | `Kill` | Send signal to process |
| 17 | `Sigreturn` | Return from signal handler |
| 18 | `Socket` | Create network socket |
| 19 | `Bind` | Bind socket to address/port |
| 20 | `Listen` | Listen for socket connections |
| 21 | `Accept` | Accept incoming socket connection |
| 22 | `Connect` | Connect socket to remote address |
| 23 | `GetTimeOfDay` | Read system clock |
| 24 | `SetTimeOfDay` | Set system clock |
| 25 | `OtaState` | Query and manage OTA partitions |
| 26 | `Pipe` | Create unidirectional IPC pipe |
| 27 | `Dup2` | Duplicate file descriptor |
| 28 | `Chdir` | Change working directory |
| 29 | `Getcwd` | Get current working directory |

---

### 3.5 Virtual File System (VFS), /dev, /proc, /sys, Pipes & Sockets

- **Core VFS Architecture** (`kernel/src/vfs/`): Represented by the `Inode` trait. Open flags include `RDONLY` (0x1), `WRONLY` (0x2), `RDWR` (0x3), `CREATE` (0x100), `APPEND` (0x200), and `TRUNC` (0x400). Per-process file descriptor table supports up to 64 open files per process.
- **DevFs Nodes (`/dev`)**:
  - `/dev/null`, `/dev/zero`: Null and zero generators.
  - `/dev/console`: Interactive console (UART or SSH session).
  - `/dev/i2c0`: Master I2C bus node.
  - `/dev/spi0`: Master SPI bus node.
  - `/dev/wlan0`: Wi-Fi adapter control interface.
  - `/dev/sha0`: Hardware SHA-256 accelerator.
  - `/dev/power`: System power and reset controller.
  - `/dev/ble0`: Bluetooth Low Energy controller.
- **ProcFs Synthesized Files (`/proc`)**:
  - `/proc/uptime`: Uptime in seconds.
  - `/proc/meminfo`: Memory usage breakdown (SRAM heap, PSRAM heap, PSRAM exec slots).
  - `/proc/stacks`, `/proc/tasks`: Complete kernel task listing, states, and stack watermarks.
  - `/proc/net/sockets`: Active network socket table.
  - `/proc/<pid>/status`: Process-specific status metrics.
- **SysFs Synthesized Files (`/sys`)**:
  - `/sys/kernel`: Kernel identification and version string.
  - `/sys/smp`: Multiprocessing status.
  - `/sys/pms`: Hardware memory protection status.
- **Pipes** (`kernel/src/vfs/pipe.rs`): In-memory IPC pipes supporting blocking reads/writes and task notification.
- **Sockets** (`kernel/src/vfs/socket.rs`): Userland socket interface over `smoltcp` for TCP and UDP.
- **Flash & Memory Filesystems**:
  - **EspFs** (`kernel/src/fs/espfs/`): Log-structured filesystem on NOR flash with atomic superblocks and compaction ping-pong.
  - **ramfs** (`kernel/src/fs/ramfs.rs`): In-memory volatile filesystem backing `/tmp` and root `/` fallback.

---

### 3.6 Device Drivers & ioctl Bus Pattern

Data-carrying drivers follow a unified architecture pattern: `open("/dev/<node>")` followed by `ioctl(cmd, arg)` where `arg` points to a `#[repr(C)]` request struct. The kernel validates the struct and internal pointers using `validate_user(ptr, len)`.

| Node `/dev` | Init Hardware | ioctl Commands & Bounds | Status |
| :--- | :--- | :--- | :--- |
| `i2c0` | GPIO8 (SDA), GPIO9 (SCL) | `I2C_PROBE` (0), `I2C_READ` (1), `I2C_WRITE` (2). `I2cReq` struct (max 64 bytes buffer). | Functional |
| `spi0` | GPIO12 (MOSI), GPIO11 (MISO), GPIO13 (CLK) | `SPI_TRANSFER` (0). `SpiReq` struct (max 64 bytes buffer). | Functional |
| `sha0` | Hardware SHA engine | `SHA256_CMD` (0). `ShaReq` struct (max 512 bytes input, 32 bytes output). | Hardware-Verified |
| `power` | LPWR Controller | `POWER_SLEEP` (0), `POWER_DEEP_SLEEP` (1), `POWER_REBOOT` (2). | Reboot Functional |
| `wlan0` | `esp-wifi` STA | `WLAN_NOP` (0), `WLAN_CONNECT` (1), `WLAN_DISCONNECT` (2), `WLAN_SCAN` (3). | Hardware-Verified |
| `ble0` | `esp-wifi` BLE | `BLE_ADVERTISE` (0). Queues atomic advertising request. | Hardware-Verified |

---

### 3.7 Authentication System & passwd Command

The system integrates identity management and user authentication for the SSH server and user shell control:

#### `passwd` Command Implementation
- **Source Location**: `userland/apps/src/bin/passwd.rs`.
- **Usage Syntax**:
  - `passwd NEW_PASSWORD`: Sets the password for the default user (`youareme`).
  - `passwd USER NEW_PASSWORD`: Sets the password for the specified user name.
- **Input Validation**:
  - User and password strings must be non-empty.
  - User and password strings cannot contain `:` or newline `\n` characters.
- **Filesystem Mapping**:
  - Opens `/etc/passwd` with flags `O_WRONLY_CREATE_TRUNC` (`0x0502`).
  - Writes single-line format `user:password\n` and closes the descriptor.
  - Stored on EspFs flash storage, making the credential change **persistent across reboots and re-flashes**.

#### Kernel SSH Authentication Verification
- **Source Location**: `kernel/src/drivers/ssh/auth.rs` (`check_password`).
- **Validation Flow**:
  1. On SSH password login request, the server attempts to resolve `/etc/passwd` via VFS (`vfs::mount::resolve("/etc/passwd")`).
  2. If `/etc/passwd` exists, it reads the file and matches the target user line.
  3. Password verification uses constant-time comparison (`ct_eq` via `subtle` crate) to prevent timing attacks.
  4. If `/etc/passwd` does not exist or the user is not found, authentication falls back to compiled dev credentials (`DEV_USER` = `"youareme"`, `DEV_PASSWORD` in `kernel/src/drivers/ssh/config.rs`).

#### Boot Warnings & Credentials Reset
- **Kernel Boot Behavior** (`kernel/src/main.rs`):
  - The system never seeds a default `/etc/passwd` on boot.
  - If `/etc/passwd` is detected at boot, the kernel issues a serial console warning:
    `[kernel] WARNING: /etc/passwd exists and overrides the compiled SSH credential; 'rm /etc/passwd' to fall back to it`.
  - Executing `rm /etc/passwd` from the shell removes the file and reverts SSH authentication to compiled defaults.

---

### 3.8 Userland, ELF Loader & /bin/sh Shell

- **Userland Compilation**: Located in `userland/`. Compiled as an independent workspace with linker scripts generated by `kernel/build.rs`.
- **Load-Time Relocation**: Xtensa LLVM backend lacks PIC/PIE support. EspressoOS compiles binaries as static executables and extracts `R_XTENSA_32` data relocations via `ld --emit-relocs`. `build.rs` appends a fixup table trailer to each ELF. On `spawn`, the loader (`kernel/src/fs/elf.rs`) copies the binary into a free PSRAM slot and patches data literal pointers without altering CPU instruction bytes.
- **`libc` Library** (`userland/libc/`): Provides entry `_start`, panic handler, 32 KB non-reclaiming bump allocator, and typed wrappers for all 30 syscalls.
- **Userland Shell (`/bin/sh`)**:
  - Interactive shell located at `/bin/sh`.
  - Supports multi-stage pipelines (`|`), input/output redirection (`>`, `>>`), command sequencing (`;`), and quote parsing.
  - Built-in commands: `cd`, `pwd`, `clear`, `sudo` (no-op prefix), `exit`, `help`.

---

## 4. Build & Flash Guide

### 4.1 Prerequisites

Install required Espressif Rust tools:

```bash
cargo install espup --locked
cargo install espflash@3.3.0 --locked
espup install
```

On Windows (PowerShell):
```powershell
. $HOME/export-esp.ps1
```

On Linux / macOS:
```bash
source $HOME/export-esp.sh
```

> IMPORTANT: `espflash` version 3.3.0 is required. Version 4.x requires ESP-IDF App Descriptors incompatible with the `esp-hal` version (0.23.1) used in this repository.

### 4.2 Fallback Wi-Fi Configuration (Optional)

To configure secondary compiled fallback Wi-Fi credentials:

```bash
cp kernel/src/wifi_credentials.rs.example kernel/src/wifi_credentials.rs
```

Edit `WIFI_SSID` and `WIFI_PASSWORD` in `kernel/src/wifi_credentials.rs`. Saved NVS credentials configured via `/bin/wifi connect` will always take precedence over compiled defaults.

### 4.3 Build and Flash

To build the kernel and flash the board:

```bash
cargo build --release
cargo run --release
```

`kernel/build.rs` automatically compiles all 35 userland binaries, generates relocation fixup tables, and embeds them into the kernel image.

### 4.4 Feature Gates

The following feature flags are available in `kernel/Cargo.toml`:

| Feature Flag | Default State | Description |
| :--- | :--- | :--- |
| `syscall-trap` | Enabled | Enables CPU trap handling for system calls via `syscall` instruction (EXCCAUSE=1). |
| `smp` | Disabled | Enables dual-core execution on ProCpu and AppCpu. |
| `pms` | Disabled | Enables hardware memory protection monitor and stack guards. |
| `diag-ble-sync` | Disabled | Diagnostic feature for synchronous BLE execution. |
| `diag-32k-stack` | Disabled | Increases default task stack size to 32 KB for diagnostic testing. |

### 4.5 Expected serial output

```text
[kernel] PSRAM added to heap: 7340032 bytes @ 0x3c1f0000 (1MB reserved for Userland @ 0x3c0f0000)
[psram-exec] reserved PSRAM mapped to the instruction bus @ 0x42800000 (16 pages)
[psram-exec] OK: code EXECUTED from PSRAM returned 42 (expected 42)

========================================
   EspressoOS   ·   kernel
   Live console. Starting subsystems.
========================================
[kernel] flash: 16 MB usable
[kernel] / mounted on flash (espfs)
[kernel] userland: 32 binaries installed/updated in EspFs
[net] connecting to SSID '...'
[net] associated with AP; negotiating DHCP...
[net] IP = 192.168.2.146
[net] SSH listening on port 22, ECHO on 2323, OTA on 3300

EspressoOS:~$
```

---

## 5. Complete Command Reference

The system includes **35 userland binaries** in `/bin` alongside shell built-in commands.

### Userland Binaries (`/bin/*`)

1. **`init`**: Supervisor process executing `/etc/rc` and launching interactive `/bin/sh`.
2. **`sh`**: Primary interactive command shell with pipe, redirection, and variable support.
3. **`cat`**: Display file contents or stream standard input.
4. **`echo`**: Print text arguments (`echo [-n] TEXT`).
5. **`ls`**: List directory contents.
6. **`mkdir`**: Create new directories.
7. **`touch`**: Create empty files or update file presence.
8. **`rm`**: Remove files from the filesystem.
9. **`write`**: Write text to file, overwriting existing contents.
10. **`passwd`**: Create or update `/etc/passwd` to modify SSH credentials (`passwd [USER] PASSWORD`).
11. **`wifi`**: Wi-Fi management utility (`wifi status`, `wifi scan`, `wifi connect "SSID" "PASS"`, `wifi disconnect`).
12. **`ip`**: Display `wlan0` IP address, SSID, and link status.
13. **`nmcli`**: Network control CLI compatible with nmcli syntax.
14. **`ping`**: Send ICMP Echo Requests to remote IPv4 target (`ping IP`).
15. **`tcping`**: Probe TCP connectivity and measure round-trip time (`tcping IP [PORT]`).
16. **`sntp`**: Synchronize system clock via SNTP protocol (`sntp [SERVER_IP]`).
17. **`httpd`**: HTTP/1.1 web server exposing `/proc/uptime` and `/proc/meminfo` metrics (`httpd [PORT]`).
18. **`netstat`**: Print active socket table from `/proc/net/sockets`.
19. **`uptime`**: Show system uptime.
20. **`free`**: Report heap memory usage and PSRAM execution slot metrics.
21. **`ps`**: List active tasks and processes.
22. **`kill`**: Send signals to running processes (`kill -SIGKILL PID`).
23. **`sha256`**: Calculate SHA-256 digest using hardware acceleration engine.
24. **`power`**: Manage system power modes (`power sleep`, `power deep-sleep`, `power reboot`).
25. **`reboot`**: Trigger software reset.
26. **`ble`**: Control Bluetooth Low Energy advertiser (`ble status`, `ble advertise`).
27. **`i2c`**: I2C bus diagnostic tool (`i2c scan`, `i2c read`, `i2c write`).
28. **`spi`**: Full-duplex SPI transfer tool (`spi transfer`).
29. **`ota`**: OTA A/B firmware update utility (`ota status`, `ota mark-valid`, `ota rollback`).
30. **`smp`**: Display multiprocessor status.
31. **`pms`**: Display memory protection status.
32. **`sleep`**: Pause execution for specified duration.
33. **`badptr`**: Test utility triggering invalid memory accesses to verify kernel fault handling.
34. **`cwdtest`**: Test utility verifying directory change and getcwd behavior.
35. **`ioctltest`**: Stress test and bounds check utility for `ioctl` interface.

---

## 6. The Shells — Prompts, Redirection, Pipes & Sessions

### System Prompts
- Serial Console (`/bin/sh`): `EspressoOS:~$`
- SSH Session (`/bin/sh` over SSH): `youareme@EspressoOS:~$` (or authenticated username).

### Redirection
Output redirection uses `>` (truncate) and `>>` (append), modifying file descriptor tables via `dup2`.

Example:
```bash
echo "Server active" > /tmp/log.txt
cat /tmp/log.txt
```

### Pipes
Pipes connect standard output of one process to standard input of the next using `|`.

Example:
```bash
/bin/ps | /bin/cat
/bin/echo "Test string" | /bin/sha256
```

### Session Management
Each serial or SSH connection runs an isolated session (`SessionChannel`) with dedicated file descriptors and working directory. Disconnecting frees file descriptors and sweeps associated child processes.

### Userland programs (`/bin`)

See the full 32-binary table in §3.7. Selected usage:

```text
EspressoOS:~$ /bin/echo hola mundo | /bin/cat
hola mundo
EspressoOS:~$ /bin/ls /bin | /bin/cat
EspressoOS:~$ sha256 hello           # 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
EspressoOS:~$ wifi connect "My Home Net" "password"
EspressoOS:~$ free
            total         used         free
heap      7471104       171312      7299792
slots          32            0           32
```

---

## 7. Repository Structure

```text
EspressoOS/
├── .cargo/
│   └── config.toml          # Xtensa target config and espflash runner setup
├── bootloader/              # Second-stage bootloader skeleton (future phase)
├── kernel/                  # EspressoOS Kernel crate
│   ├── build.rs             # Build script: compiles 35 userland ELFs and generates fixup tables
│   ├── src/
│   │   ├── arch/xtensa/     # Context switch, ISR vectors, and SYSTIMER driver
│   │   ├── drivers/         # Drivers: gpio, uart, i2c, spi, crypto, flash, power, ble, wifi, ssh
│   │   ├── fs/              # Filesystems: espfs, ramfs, procfs, sysfs, littlefs, elf.rs
│   │   ├── mm/              # Memory management: heap, psram_exec (slots), mpu (pms)
│   │   ├── ota/             # OTA A/B partition management
│   │   ├── scheduler/       # Multitasking scheduler, tasks, Round-Robin policy, processes
│   │   ├── shell/           # Kernel fallback shell module
│   │   ├── syscall/         # Syscall table (0..29), handlers, and trap dispatch
│   │   ├── vfs/             # Virtual File System: inodes, devfs, pipes, sockets, mounts
│   │   ├── main.rs          # Kernel entry point and boot sequencer
│   │   ├── prelude.rs       # Memory layout, system constants, and prelude imports
│   │   └── session.rs       # Session channels (UART / SSH)
│   └── Cargo.toml
├── tools/                   # Partition generation tools and test scripts
├── userland/                # no_std userland workspace (executes from PSRAM)
│   ├── apps/src/bin/        # Source code for 35 userland binaries (/bin/*)
│   ├── libc/                # Userland base library: _start, syscall wrappers, allocator
│   └── Cargo.toml
├── Cargo.lock               # Cargo workspace lockfile
├── Cargo.toml               # Main Cargo workspace manifest
├── espflash.toml            # Flash size (16MB) and partition table configuration
├── LICENSE                  # Project MIT License
├── partitions.csv           # 16 MB flash partition layout table
└── rust-toolchain.toml      # Toolchain configuration (esp channel)
```

---

## 8. Memory Map & Partition Table

### 16 MB Flash Memory Layout
Flash layout defined in `kernel/src/prelude.rs` and `partitions.csv`:

```text
0x00000000 - 0x00007FFF : Second-stage Bootloader (32 KB)
0x00008000 - 0x00008FFF : Partition Table (4 KB)
0x00009000 - 0x0000EFFF : NVS Storage (24 KB - Saved Wi-Fi credentials)
0x0000F000 - 0x0001FFFF : OTA Control Data otadata (68 KB)
0x00020000 - 0x0041FFFF : Factory Partition / Slot A (Primary kernel, 4 MB)
0x00420000 - 0x0081FFFF : OTA_0 Partition / Slot B (Secondary kernel, 4 MB)
0x00820000 - 0x00FEFFFF : EspFs Filesystem (7.8 MB)
0x00FF0000 - 0x00FFFFFF : CoreDump Partition (64 KB)
```

### SRAM and PSRAM Memory Layout
- **Internal SRAM (512 KB)**: 128 KB allocated to internal kernel heap; remainder used for system stack and vendor Wi-Fi/Bluetooth buffers.
- **External Octal PSRAM (8 MB)**:
  - ~7 MB dedicated to dynamic kernel heap.
  - 1 MB reserved for userland execution pool (`psram_exec`):
    - Executable text mapping on instruction bus: `0x42800000` (512 KB).
    - Data mapping on data bus: `0x3c170000` (512 KB).
    - 32 slots of 16 KB.

---

## 9. Known Issues & Technical Debt

1. **Light Sleep (`power sleep`)**: Light sleep hangs the CPU due to underlying platform limitations. Use `power reboot` or `power deep-sleep`.
2. **EspFs Compaction Limit**: EspFs log-structured compaction requires free flash space. Extremely low flash conditions may return a `NoSpace` error during compaction.
3. **Plaintext Password Storage**: `/etc/passwd` stores passwords in plaintext. While SSH verification uses constant-time string comparisons (`ct_eq`) to mitigate timing attacks, storage does not currently use salted password hashing algorithms (such as bcrypt or SHA-512).
4. **Non-Reentrant Kernel Mutex**: Kernel `Mutex` disables CPU interrupts while held. Acquiring the same mutex twice on the same core will result in a silent dead lock.

---

## License

Licensed under the **MIT License**. See [LICENSE](LICENSE).

## Contact

Author: **Jorge Gaspar Beltre Rivera**  
Project: **EspressoOS — A `no_std` Unix-like Operating System in Rust for ESP32-S3**


<p align="center">
  <a href="https://www.linkedin.com/in/jorge-gaspar-beltre-rivera/" target="_blank"><img src="https://user-images.githubusercontent.com/74038190/235294012-0a55e343-37ad-4b0f-924f-c8431d9d2483.gif" alt="LinkedIn" width="100"></a>
  <a href="https://github.com/JorgeGBeltre" target="_blank"><img src="https://user-images.githubusercontent.com/74038190/212257468-1e9a91f1-b626-4baa-b15d-5c385dfa7ed2.gif" alt="GitHub" width="100"></a>
  <a href="mailto:Jorgegaspar3021@gmail.com"><img src="https://user-images.githubusercontent.com/74038190/216122065-2f028bae-25d6-4a3c-bc9f-175394ed5011.png" alt="E-Mail" width="100"></a>

</p>

## Support

This project is developed independently. Even a small contribution helps me dedicate more time to development, testing, and releasing new features.


 <p align="center">
  <a href="https://www.paypal.com/donate/?hosted_button_id=2VLA8BWT967LU">
    <img src="https://www.paypalobjects.com/webstatic/icon/pp258.png"
         alt="Donate with PayPal"
         height="60">
  </a>
</p>
