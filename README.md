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
- [10. Changelog & Implementation History](#10-changelog--implementation-history)
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
- **Preemptive Multitasking & Process Management**: Scheduler driven by SYSTIMER at 100 Hz, 50 ms quantum, and Model B in-ISR context switching. Automatic reparenting of orphan processes to PID 1 (`init-sup`) and non-blocking zombie reaping with `waitpid` (`WNOHANG`), preventing PSRAM-Exec slot leaks.
- **Dynamic Userland ELFs**: Suite of **140 ELF binaries** executing from PSRAM with `argv` support, 32-slot pool, and load-time relocation without requiring Position Independent Code (PIC).
- **Wi-Fi + TCP/IP Stack**: Station (STA) mode, DHCP client, and active TCP/IP stack (`smoltcp`). Runtime Wi-Fi scan, connect, and disconnect. Credential persistence in flash NVS sector 0 (`0x0000_9000`).
- **Hardened SSH-2.0 Server**: Server supporting curve25519-sha256, ssh-ed25519, and chacha20-poly1305@openssh.com.
  - **Persistent Host Key**: Unique Ed25519 host key generated via hardware TRNG on first boot and persisted to NVS flash sector 1 (`0x0000_A000`, `ESHK` format).
  - **Public-Key Authentication**: Supports OpenSSH `ssh-ed25519` keys in `/etc/authorized_keys` with constant-time cryptographic verification (`ct_eq`).
  - **Connection Resilience**: 60s authentication deadline, 300s session idle timeout, and clean TCP socket teardown on remote FIN/RST (`CloseWait`).
- **`passwd` Command & Credential Management**: Userland application `/bin/passwd` creates or updates `/etc/passwd` in persistent flash storage (EspFs) using salted SHA-256 (`$s5$<salt>$<hash>`).
- **Persistent EspFs Filesystem & VFS**: Log-structured filesystem on NOR flash surviving reboots and re-flashes, with dynamic mount table and ramfs scratch mount on `/mnt`.
- **Hardware Cryptography & TRNG**: Drivers `/dev/sha0` exposing the hardware SHA-256 accelerator, and `/dev/random` + `/dev/urandom` streaming entropy from the on-chip True Random Number Generator.
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
- **Light Sleep**: Removed (`power sleep` no longer exists). esp-hal's `rtc.sleep_light()` hangs the CPU on this hardware/HAL generation; `power deep-sleep` and `power reboot` cover the same use cases without that risk.
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
   - Mount `/tmp` (ramfs), `/proc` (procfs), `/sys` (sysfs), `/mnt` (ramfs).
6. **PID 0 File Descriptor Table**: Seed `/dev/console` for standard streams (fd 0, 1, 2).
7. **Userland Deployment**: `install_userland()` inspects the 63 embedded binaries and deploys or updates `/bin/*` on EspFs.
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
  - `/dev/gpio`: GPIO controller for pins 0..48.
  - `/dev/watchdog`: Hardware Watchdog Timer interface.
  - `/dev/pwm0`: LEDC hardware PWM controller (8 channels).
  - `/dev/adc0`: SAR ADC analog reader (10 channels, GPIO 1..10).
- **ProcFs Synthesized Files (`/proc`)**:
  - `/proc/uptime`: Uptime in milliseconds.
  - `/proc/meminfo`: Memory usage breakdown (SRAM heap, PSRAM heap, PSRAM exec slots).
  - `/proc/stacks`, `/proc/tasks`: Complete kernel task listing, states, and stack watermarks.
  - `/proc/net/sockets`: Active network socket table.
  - `/proc/watchdog`: Watchdog enabled status, timeout, feed count, and boot reset reason.
  - `/proc/devices`: Registered character and block device driver table.
  - `/proc/interrupts`: CPU interrupt lines, vectors, and scheduler tick counters.
  - `/proc/partitions`: Flash partition table matching hardware memory layout.
  - `/proc/crypto`: Hardware accelerators and cryptographic cipher engines.
  - `/proc/<pid>/status`: Process-specific status metrics.
- **SysFs Synthesized Files (`/sys`)**:
  - `/sys/kernel`: Kernel identification and version string.
  - `/sys/smp`: Multiprocessing status.
  - `/sys/pms`: Hardware memory protection status.
  - `/sys/power`: CPU, APB, and XTAL clock frequencies and supported sleep modes.
- **Pipes** (`kernel/src/vfs/pipe.rs`): In-memory IPC pipes supporting blocking reads/writes and task notification.
- **Sockets** (`kernel/src/vfs/socket.rs`): Userland socket interface over `smoltcp` for TCP and UDP.
- **Flash & Memory Filesystems**:
  - **EspFs** (`kernel/src/fs/espfs/`): Log-structured filesystem on NOR flash with atomic superblocks and compaction ping-pong.
  - **ramfs** (`kernel/src/fs/ramfs.rs`): In-memory volatile filesystem backing `/tmp` and `/mnt`.
  - **LittleFs** (`kernel/src/fs/littlefs/`): Resilient hierarchical Inode-based filesystem engine supporting files, directories, read/write/truncate, and mount points.

---

### 3.6 Device Drivers & ioctl Bus Pattern

Data-carrying drivers follow a unified architecture pattern: `open("/dev/<node>")` followed by `ioctl(cmd, arg)` where `arg` points to a `#[repr(C)]` request struct. The kernel validates the struct and internal pointers using `validate_user(ptr, len)`.

| Node `/dev` | Init Hardware | ioctl Commands & Bounds | Status |
| :--- | :--- | :--- | :--- |
| `i2c0` | GPIO8 (SDA), GPIO9 (SCL) | `I2C_PROBE` (0), `I2C_READ` (1), `I2C_WRITE` (2). `I2cReq` struct (max 64 bytes buffer). | Functional |
| `spi0` | GPIO12 (MOSI), GPIO11 (MISO), GPIO13 (CLK) | `SPI_TRANSFER` (0). `SpiReq` struct (max 64 bytes buffer). | Functional |
| `sha0` | Hardware SHA engine | `SHA256_CMD` (0). `ShaReq` struct (max 512 bytes input, 32 bytes output). | Hardware-Verified |
| `power` | LPWR Controller | `POWER_DEEP_SLEEP` (1), `POWER_REBOOT` (2). ioctl 0 (former `POWER_SLEEP`) now returns `NotSupported` -- see Known Issues. | Reboot Functional |
| `passwd` | — (crypto only) | `SET_PASSWORD_CMD` (0). `PasswdReq` struct (max 64 bytes user, 128 bytes password); salts+hashes (SHA-256) before writing `/etc/passwd`. | Functional |
| `wlan0` | `esp-wifi` STA | `WLAN_NOP` (0), `WLAN_CONNECT` (1), `WLAN_DISCONNECT` (2), `WLAN_SCAN` (3). | Hardware-Verified |
| `ble0` | `esp-wifi` BLE | `BLE_ADVERTISE` (0). Queues atomic advertising request. | Hardware-Verified |
| `gpio` | GPIO Matrix (pins 0..48) | `GPIO_IOC_CONFIGURE` (0x4701), `GPIO_IOC_WRITE` (0x4702), `GPIO_IOC_READ` (0x4703), `GPIO_IOC_TOGGLE` (0x4704). Also accepts text commands on write. | Hardware-Verified |
| `watchdog` | RTC/Timer Watchdog | `WDT_IOC_FEED` (0x5701), `WDT_IOC_GET_STATUS` (0x5702), `WDT_IOC_SET_TIMEOUT` (0x5703), `WDT_IOC_ENABLE` (0x5704). Writing feeds watchdog. | Hardware-Verified |
| `pwm0` | LEDC Peripheral (8 channels) | `PWM_IOC_CONFIG` (0x5001), `PWM_IOC_SET_DUTY` (0x5002), `PWM_IOC_ENABLE` (0x5003), `PWM_IOC_DISABLE` (0x5004), `PWM_IOC_GET_STATUS` (0x5005). | Hardware-Verified |
| `adc0` | SAR ADC1 (GPIO 1..10) | `ADC_IOC_READ` (0x4101), `ADC_IOC_GET_MV` (0x4102), `ADC_IOC_SAMPLE_AVG` (0x4103). Emits channel voltages and gauges on read. | Hardware-Verified |
| `fb0` | Monochrome Framebuffer (128x64) | `FBIO_GET_VSCREENINFO` (0x4600), `FBIO_CLEAR` (0x4601), `FBIO_FILL` (0x4602), `FBIO_INVERT` (0x4603). Direct byte read/write (1024 B). | Hardware-Verified |
| `random` / `urandom` | Hardware TRNG | Read streams cryptographically secure entropy from on-chip TRNG. | Hardware-Verified |

---

### 3.7 Authentication System & passwd Command

The system integrates identity management and user authentication for the SSH server and user shell control:

#### `passwd` Command Implementation
- **Source Location**: `userland/apps/src/bin/passwd.rs` (client) + `kernel/src/drivers/passwd.rs` (`/dev/passwd`, does the actual hashing).
- **Usage Syntax**:
  - `passwd NEW_PASSWORD`: Sets the password for the default user (`youareme`).
  - `passwd USER NEW_PASSWORD`: Sets the password for the specified user name.
- **Input Validation**: enforced on both sides of the ioctl boundary (userland client and kernel driver) —
  - User and password strings must be non-empty.
  - User and password strings cannot contain `:` or newline `\n` characters.
- **Storage format**: `passwd(1)` sends `{user, password}` to `/dev/passwd` via `ioctl(SET_PASSWORD_CMD)`; the kernel generates a 16-byte salt from the hardware TRNG, computes `SHA-256(salt || password)`, and writes a single line `user:$s5$<salt-hex>$<hash-hex>\n` to `/etc/passwd` (`O_WRONLY_CREATE_TRUNC`). The plaintext password is never written to flash. Stored on EspFs flash storage, making the credential change **persistent across reboots and re-flashes**.

#### Kernel SSH Authentication & Host Key Management
- **Persistent Host Key**:
  - Generated on first boot via the hardware TRNG (`esp_hal::rng::Rng`).
  - Stored in dedicated NVS flash sector 1 (`0x0000_A000`, record magic `ESHK` + CRC checksum) so the host fingerprint is stable across reboots without throwing host key mismatch warnings on SSH clients.
  - Automatically announced on the serial console at boot: `[ssh] host key ssh-ed25519 fingerprint: SHA256:...`.
- **Public-Key Authentication (`/etc/authorized_keys`)**:
  - The SSH server parses `/etc/authorized_keys` via VFS.
  - Supports standard OpenSSH format (`ssh-ed25519 <base64-blob> [comment]`).
  - Decoded using the kernel's RFC 4648 Base64 engine (`drivers::ssh::base64`) and verified against the incoming client signature in constant time (`ct_eq`).
- **Password Authentication (`/etc/passwd`)**:
  - On SSH password login request, the server attempts to resolve `/etc/passwd` via VFS (`vfs::mount::resolve("/etc/passwd")`).
  - Matches the user in constant time (`ct_eq`). If the stored field starts with `$s5$`, it verifies `SHA-256(salt || password)`.
  - If `/etc/passwd` does not exist, it falls back to compiled developer credentials (`DEV_USER` / `DEV_PASSWORD`).
- **Connection Timeouts & Socket Teardown**:
  - **Auth Timeout**: Non-authenticated sessions terminate after 60 seconds of inactivity.
  - **Idle Timeout**: Interactive sessions terminate after 300 seconds (5 minutes) of silence.
  - **TCP FIN/RST Teardown**: Sockets in `CloseWait` or abruptly closed by the client are immediately torn down, freeing the session shell and channel.

#### Boot Warnings & Credentials Reset
- **Kernel Boot Behavior** (`kernel/src/main.rs`):
  - The system never seeds a default `/etc/passwd` or `/etc/authorized_keys` on boot.
  - If `/etc/passwd` is detected at boot, the kernel issues a serial console warning:
    `[kernel] WARNING: /etc/passwd exists and overrides the compiled SSH credential; 'rm /etc/passwd' to fall back to it`.
  - If `/etc/authorized_keys` is detected:
    `[kernel] NOTICE: /etc/authorized_keys exists and enables SSH public-key authentication`.
  - Executing `rm /etc/passwd` from the shell removes custom password credentials; `rm /etc/authorized_keys` disables public-key authentication.

---

### 3.8 Userland, ELF Loader & /bin/sh Shell

- **Userland Compilation**: Located in `userland/`. Compiled as an independent workspace with linker scripts generated by `kernel/build.rs`.
- **Load-Time Relocation**: Xtensa LLVM backend lacks PIC/PIE support. EspressoOS compiles binaries as static executables and extracts `R_XTENSA_32` data relocations via `ld --emit-relocs`. `build.rs` appends a fixup table trailer to each ELF. On `spawn`, the loader (`kernel/src/fs/elf.rs`) copies the binary into a free PSRAM slot and patches data literal pointers without altering CPU instruction bytes.
- **Process Lifecycle & Reaping**:
  - When an exited process has live child processes, they are automatically reparented to `PID 1` (`init-sup`).
  - `sys_wait` (`syscall 7`) implements `WNOHANG`, exposed via `libc::waitpid(pid, status, options)`.
  - `/bin/sh` non-blockingly reaps background jobs (`&`) before displaying each interactive prompt, eliminating zombie accumulation and PSRAM slot leaks.
- **`libc` Library** (`userland/libc/`): Provides entry `_start`, panic handler, 32 KB non-reclaiming bump allocator, and typed wrappers for all 30 syscalls.
- **Userland Shell (`/bin/sh`)**:
  - Interactive shell located at `/bin/sh`.
  - Supports multi-stage pipelines (`|`), input/output redirection (`>`, `>>`), command sequencing (`;`), background execution (`&`), and quote parsing.
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

`kernel/build.rs` automatically compiles all 122 userland binaries, generates relocation fixup tables, and embeds them into the kernel image.

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
[kernel] userland: 122 binaries installed/updated in EspFs
[net] connecting to SSID '...'
[net] associated with AP; negotiating DHCP...
[net] IP = 192.168.2.146
[ssh] host key ssh-ed25519 fingerprint: SHA256:d8Q+...
[net] SSH listening on port 22, ECHO on 2323, OTA on 3300

EspressoOS:~$
```

---

## 5. Complete Command Reference

The system includes **122 userland binaries** in `/bin` alongside shell built-in commands.

### Userland Binaries (`/bin/*`)

1. **`init`**: Supervisor process executing `/etc/rc` and launching interactive `/bin/sh`.
2. **`sh`**: Primary interactive command shell with pipe, redirection, background jobs (`&`), and variable support.
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
19. **`uptime`**: Show system uptime in seconds.
20. **`free`**: Report heap memory usage and PSRAM execution slot metrics.
21. **`ps`**: List active tasks and processes with PID, state, and name.
22. **`kill`**: Send signals to running processes (`kill -SIGKILL PID`).
23. **`sha256`**: Calculate SHA-256 digest using hardware acceleration engine.
24. **`power`**: Manage system power modes (`power deep-sleep`, `power reboot`).
25. **`reboot`**: Trigger software reset.
26. **`ble`**: Control Bluetooth Low Energy advertiser (`ble status`, `ble advertise`).
27. **`i2c`**: I2C bus diagnostic tool (`i2c scan`, `i2c read`, `i2c write`).
28. **`spi`**: Full-duplex SPI transfer tool (`spi transfer`).
29. **`ota`**: OTA A/B firmware update utility (`ota status`, `ota mark-valid`, `ota rollback`).
30. **`smp`**: Display multiprocessor status.
31. **`pms`**: Display memory protection status.
32. **`sleep`**: Pause execution for specified duration (`sleep SECONDS`).
33. **`badptr`**: Test utility triggering invalid memory accesses to verify kernel fault handling.
34. **`cwdtest`**: Test utility verifying directory change and getcwd behavior.
35. **`ioctltest`**: Stress test and bounds check utility for `ioctl` interface.
36. **`cp`**: Copy file contents from source to destination using chunked buffer streaming (`cp SRC DST`).
37. **`head`**: Print first N lines of standard input or files (`head [-n N] [FILE...]`).
38. **`tail`**: Print last N lines using a circular ring buffer (`tail [-n N] [FILE...]`).
39. **`wc`**: Print newline, word, and byte counts for files or stdin (`wc [FILE...]`).
40. **`grep`**: Print lines matching a specified pattern in files or stdin (`grep PATTERN [FILE...]`).
41. **`df`**: Report filesystem disk space usage, block sizes, and mount points (`df`).
42. **`dmesg`**: Dump kernel circular diagnostic ring buffer with microsecond timestamps (`dmesg`).
43. **`date`**: Format and display current system uptime clock (`date`).
44. **`sync`**: Explicitly flush in-memory buffers to persistent flash storage (`sync`).
45. **`nslookup`**: Query Internet domain name servers via UDP RFC 1035 (`nslookup HOSTNAME [DNS_SERVER]`).
46. **`curl`**: HTTP/1.1 client for fetching web endpoints over TCP (`curl [-i] IP [PORT] [PATH]`).
47. **`mv`**: Move or rename files across filesystem locations (`mv SOURCE DESTINATION`).
48. **`stat`**: Display file and directory metadata, byte size, and inode type (`stat PATH`).
49. **`tee`**: Duplicate standard input to standard output and files (`tee [-a] FILE`).
50. **`env`**: Print system environment variables, working directory, and OS release (`env`).
51. **`clear`**: Clear terminal screen and position cursor to origin using ANSI escape sequence (`clear`).
52. **`pwd`**: Print current working directory path from process context (`pwd`).
53. **`poweroff`**: Perform clean hardware shutdown by placing the board into deep sleep (`poweroff`).
54. **`mount`**: Display active VFS mount table, device mappings, and filesystem types (`mount`).
55. **`umount`**: Unmount filesystem or report busy/system-managed status (`umount PATH`).
56. **`less`**: Screen-by-screen file and stream pager with interactive navigation controls (`less [FILE]`).
57. **`find`**: Recursively search directory hierarchy with BFS queue and pattern matching (`find [PATH] [-name PATTERN]`).
58. **`du`**: Report recursive disk usage in blocks or human-readable format (`du [-h] [-s] [PATH]`).
59. **`service`**: System service and daemon manager with lifecycle commands (`service [list | status | start | stop | restart]`).
60. **`pkg`**: System package manager inspecting binaries, versions, and integrity (`pkg [list | info | check]`).
61. **`which`**: Locate executable binary in `/bin` command path (`which PROGRAM`).
62. **`true`**: Standard Unix utility returning exit code 0 (`true`).
63. **`false`**: Standard Unix utility returning exit code 1 (`false`).
64. **`wdt`**: Watchdog timer control and diagnostics (`wdt [status | ping | feed [N]]`).
65. **`gpio`**: GPIO pin mode configuration, digital read, write, and toggle (`gpio [mode | write | read | toggle | status] [PIN] [VAL]`).
66. **`ifconfig`**: Unix standard network interface status and configuration (`ifconfig [interface] [-a]`).
67. **`uname`**: Print system information, kernel release, and hardware architecture (`uname [-a | -s | -n | -r | -v | -m]`).
68. **`hostname`**: Print or set system network host name (`hostname [NAME]`).
69. **`top`**: Real-time system monitor reporting CPU clock, memory statistics, and task table (`top [-n ITERATIONS]`).
70. **`whoami`**: Print effective user identity name (`whoami`).
71. **`cut`**: Extract delimiter-separated fields from lines in files or stdin (`cut -d DELIM -f FIELD [FILE]`).
72. **`tr`**: Translate or delete characters from standard input stream (`tr [-d] SET1 [SET2]`).
73. **`base64`**: Encode or decode data using RFC 4648 standard (`base64 [-d] [FILE]`).
74. **`hexdump`**: Display file or stdin stream in canonical hexadecimal and ASCII representation (`hexdump [-C] [-n LENGTH] [FILE]`).
75. **`adc`**: ESP32-S3 SAR ADC reader for channels 0..9 (GPIO 1..10) with raw 12-bit readout, voltage conversion (mV), and ASCII level gauge (`adc [channel | -a]`).
76. **`pwm`**: LEDC hardware PWM controller for 8 channels (0..7) with frequency and duty cycle configuration (`pwm [status | config <ch> <pin> <hz> <duty> | duty <ch> <val> | off <ch>]`).
77. **`cal`**: Standard Unix calendar utility printing formatted month grids and calculating day-of-week alignment (`cal [month] [year]`).
78. **`basename`**: Strip directory and suffix from filenames (`basename <path> [suffix]`).
79. **`dirname`**: Strip last component from file name (`dirname <path>`).
80. **`lsof`**: List open files, active file descriptors, network sockets, and mounted filesystems (`lsof`).
81. **`mkfifo`**: Create named pipes (FIFOs) in the virtual filesystem (`mkfifo <path>`).
82. **`cmp`**: Byte-by-byte file comparison with divergence byte and line reporting (`cmp <file1> <file2>`).
83. **`sort`**: Sort lines of text files with reverse and numeric options (`sort [-r] [-n] [file]`).
84. **`uniq`**: Report or filter out repeated lines with count and duplicate flags (`uniq [-c] [-d] [file]`).
85. **`logger`**: Add structured entries to system log with facility and priority tags (`logger [-p pri] [-t tag] message...`).
86. **`xargs`**: Build and execute command lines from standard input streams (`xargs [-n max-args] [command [initial-args]]`).
87. **`cron`**: Periodic task scheduler and crontab manager (`cron [list | add <sec> <cmd> | run | help]`).
88. **`watch`**: Repeatedly execute a command at periodic intervals with ANSI screen refresh (`watch [-n interval] [-c count] <command>`).
89. **`strings`**: Extract printable ASCII character sequences from files and streams (`strings [-n min-len] [file]`).
90. **`mktemp`**: Generate unique temporary files and directories in `/tmp` (`mktemp [-d] [template]`).
91. **`ipcs`**: Report active Inter-Process Communication facilities, named pipes, and socket endpoints (`ipcs`).
92. **`timeout`**: Run a command with an execution time limit and SIGKILL termination (`timeout <duration_sec> <command> [args...]`).
93. **`ed`**: Interactive Unix line-oriented text editor (`ed [file]`).
94. **`id`**: Print real and effective user and group identification numbers (`id [-u | -g | -un | -gn]`).
95. **`su`**: Switch session user identity and spawn user shell environment (`su [username]`).
96. **`tar`**: Tape archive manipulation tool supporting creation, listing, and extraction (`tar [-c | -t | -x] <archive> [files...]`).
97. **`vmstat`**: Virtual memory, process scheduling, and system activity reporter (`vmstat [-d delay] [count]`).
98. **`diff`**: Compare files line-by-line with unified difference indicators (`diff <file1> <file2>`).
99. **`test`**: Evaluate conditional expressions and file attributes (`test <expr>` or `[ <expr> ]`).
100. **`expr`**: Evaluate mathematical and logical expressions (`expr <num1> <op> <num2>`).
101. **`patch`**: Apply unified diff files directly to source files (`patch <file> <diff_file>`).
102. **`arp`**: Display or manipulate the system ARP neighbor table (`arp [-a] [-n] [-d <host>] [-s <host> <hw_addr>]`).
103. **`logrotate`**: Rotate and archive system log files based on file size thresholds (`logrotate [-f] [-v] [logfile]`).
104. **`fbtest`**: Framebuffer graphics diagnostic and rendering test utility (`fbtest`).
105. **`nc`**: Arbitrary TCP connection and listener utility (`nc [-l] [-p port] [host] [port]`).
106. **`printenv`**: Print all or specific environment variable values (`printenv [VAR...]`).
107. **`comm`**: Compare two sorted files line by line with three-column output (`comm [-123] <file1> <file2>`).
108. **`nl`**: Number lines of files with right-aligned formatting (`nl [-ba] [-bt] [-w width] [file]`).
109. **`fold`**: Wrap input lines to fit in a specified column width (`fold [-w width] [-s] [file]`).
110. **`split`**: Split a file into fixed-size byte or line pieces (`split [-b bytes] [-l lines] [file [prefix]]`).
111. **`yes`**: Repeatedly output a line with all specified strings, or 'y' (`yes [-n count] [string]`).
112. **`rev`**: Reverse lines character-wise (`rev [file]`).
113. **`paste`**: Merge lines of files side-by-side (`paste [-d delim] <file1> <file2>`).
114. **`expand`**: Convert tabs to spaces (`expand [-t tabstop] [file]`).
115. **`unexpand`**: Convert spaces to tabs (`unexpand [-t tabstop] [file]`).
116. **`factor`**: Print prime factors of numbers (`factor [number...]`).
117. **`seq`**: Print sequences of numbers with configurable increments, custom separators, and zero-padding (`seq [-s sep] [-w] [first [incr]] last`).
118. **`nohup`**: Run commands immune to hangups with output redirection to `nohup.out` (`nohup COMMAND [ARG...]`).
119. **`shuf`**: Generate random permutations of lines, arguments, or number ranges using Fisher-Yates shuffling (`shuf [-e] [-i lo-hi] [-n count] [file]`).
120. **`od`**: Dump files in octal, character, hex, or decimal representations (`od [-b | -c | -x | -d | -o] [file...]`).
121. **`realpath`**: Resolve and canonicalize relative file and directory paths against current working directory (`realpath [-q] [-s] file...`).
122. **`cksum`**: Compute POSIX standard 32-bit CRC checksum and byte counts for files or standard input (`cksum [file...]`).
123. **`time`**: Run programs and summarize real execution time in seconds (`time COMMAND [ARG...]`).
124. **`readlink`**: Print value of a symbolic link or canonical file name (`readlink [-f] [-n] [-q] FILE...`).
125. **`tty`**: Print the file name of the terminal connected to standard input (`tty [-s]`).
126. **`groups`**: Print group memberships for a user or current login session (`groups [USERNAME...]`).
127. **`hostid`**: Print the 32-bit numeric identifier in hexadecimal for the current host (`hostid`).
128. **`dc`**: Classic Unix Desk Calculator for reverse-Polish arithmetic evaluation (`dc [-e expr] [expr]`).
129. **`sysctl`**: Configure or view kernel parameters at runtime (`sysctl [-a] [-w name=value] [name...]`).
130. **`nproc`**: Print the number of processing units available to the current process (`nproc [--all]`).
131. **`logname`**: Print the user's login name (`logname`).
132. **`chroot`**: Run commands or shell with root directory set to a new path (`chroot NEWROOT [COMMAND [ARG...]]`).
133. **`pr`**: Paginate and format text files or streams for printing (`pr [-h header] [-l lines] [-t] [file...]`).
134. **`bc`**: Classic Unix infix arithmetic calculator supporting operator precedence, parentheses, and powers (`bc [-e expr] [expr...]`).
135. **`fmt`**: Reformat text paragraphs to specified column width (`fmt [-w width] [-s] [file...]`).
136. **`tsort`**: Topological sort utility for directed acyclic graphs and dependency pipelines (`tsort [file]`).
137. **`csplit`**: Context-based file splitter by line numbers or delimiter patterns (`csplit [-f prefix] [-n digits] file split_spec...`).
138. **`nice`**: Run commands with modified scheduling priority or query current niceness (`nice [-n increment] [command [args...]]`).
139. **`dos2unix`**: Convert DOS/Windows CRLF line breaks to Unix LF line breaks (`dos2unix [-q] [file...]`).
140. **`unix2dos`**: Convert Unix LF line breaks to DOS/Windows CRLF line breaks (`unix2dos [-q] [file...]`).

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

See the full 140-binary list in §5. Selected usage:

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
│   ├── build.rs             # Build script: compiles 134 userland ELFs and generates fixup tables
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
│   ├── apps/src/bin/        # Source code for 134 userland binaries (/bin/*)
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
0x00009000 - 0x0000EFFF : NVS Storage (24 KB - Wi-Fi creds @ 0x9000, SSH Host Key @ 0xA000)
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

1. **Light Sleep & Low-Power CPU WFI Idle — IMPLEMENTED**: While platform-level RTC light sleep (`rtc.sleep_light()`) remains restricted due to octal PSRAM clock domain suspension risks on this HAL generation, safe low-power CPU sleep is now fully implemented via hardware Wait-For-Interrupt (`waiti 0`). The scheduler's idle task puts the Xtensa LX7 CPU into clock-gated low-power mode while awaiting timer ticks or hardware interrupts instead of executing a hot spin loop. Additionally, `/dev/power` exposes `POWER_SLEEP` and `POWER_IDLE` ioctls, and the userland utility `/bin/power sleep [SECONDS]` cleanly sleeps the CPU for the requested duration without resetting the board or hanging the bus.
2. **EspFs Compaction Sparse Hole Preservation — FIXED**: Log-structured filesystem compaction previously materialized sparse holes as explicit zero-filled flash records and assigned distinct extent headers for every chunk, risking size expansion on heavily fragmented or sparse files. Compaction now actively inspects extent data buffers: all-zero chunk extents are omitted from flash writes, preserving sparse holes. If the file contains trailing zero regions or is completely sparse, a lightweight 8-byte `Truncate` log record is appended to preserve the logical file size upon reboot/replay without consuming data payload flash sectors.
3. **Plaintext Password Storage — FIXED**: `/etc/passwd` stores `user:$s5$<salt-hex>$<hash-hex>` (SHA-256 of a 16-byte random salt + password), written by `/dev/passwd` (`kernel/src/drivers/passwd.rs`) so hashing occurs kernel-side. `ssh::auth::check_password` re-hashes and compares in constant time (`ct_eq`).
4. **Shared SSH Host Key — FIXED**: The SSH daemon no longer relies on a shared hardcoded development seed. On initial boot, the kernel generates a cryptographically secure 32-byte Ed25519 seed from the hardware TRNG (`esp_hal::rng::Rng`) and stores it in dedicated NVS flash sector 1 (`0x0000_A000`, `ESHK` format + CRC). Subsequent boots read the stored seed, ensuring consistent host fingerprints without client host key rotation warnings.
5. **Passwordless Public-Key Authentication — IMPLEMENTED**: OpenSSH `ssh-ed25519` keys configured in `/etc/authorized_keys` are dynamically resolved via VFS, base64-decoded, and verified in constant time (`ct_eq`), enabling passwordless SSH access.
6. **Process Slot Leaks & Reparenting — FIXED**: Orphaned child processes are automatically reparented to `PID 1` (`init-sup`). `sys_wait` implements `WNOHANG` (exposed via `libc::waitpid`), and `/bin/sh` non-blockingly reaps background jobs (`&`), preventing PSRAM-Exec slot exhaustion.
7. **Non-Reentrant Kernel Mutex Deadlock Diagnostics — FIXED**: Kernel `Mutex` disables CPU interrupts while held to protect shared resources across task contexts and interrupt handlers. To eliminate silent deadlocks when a mutex was inadvertently acquired twice on the same CPU core, atomic owner tracking was implemented. Re-entering an already held mutex on the same core is detected immediately in `lock()` and aborts with a descriptive panic indicating the core ID before spinning, preventing un-diagnosable hardware watchdog hangs. In `try_lock()`, same-core re-entrancy returns `None` safely.

---

## 10. Changelog & Implementation History


### [0.2.0-dev] - 2026-10-14

#### Added
- **Master Plan Specification Compliance**:
  - Authored 13 comprehensive architectural specifications covering subsystem maturity audit (Levels 1–5), modular architecture boundaries, layered contracts, memory layout, preemptive scheduling, 30-syscall frozen ABI, VFS interfaces, Wi-Fi/TCP networking, SSH-2.0 cryptosystem, A/B OTA partition scheme, automated testing methodology, hardware pinout mapping, and POSIX/Linux NOMMU compatibility matrix.
- **Phase 1: Process Control & Orphan Reaping (P0)**:
  - Automatic reparenting of orphaned child processes to PID 1 (`init-sup`) upon parent exit.
  - Non-blocking `WNOHANG` flag support in `wait` system call.
  - Standard `waitpid(pid, status, options)` library function in userland libc.
  - Non-blocking background job reaping (`&`) in shell before printing interactive prompts.
  - Periodic supervisor reaping loop draining adopted zombie processes, preventing PSRAM-Exec slot exhaustion.
- **Phase 2: Core POSIX Userland & Observability Expansion (P0/P1)**:
  - Expanded userland application suite to **63 dynamic ELF binaries**, all operating within 16 KB PSRAM execution slot budgets:
    - `cp`: File copy with chunked streaming buffer transfer.
    - `head`: Print first N lines from files or standard input (`-n N`).
    - `tail`: Print last N lines using a circular ring buffer (`-n N`).
    - `wc`: Multi-file line, word, and byte counter.
    - `grep`: Substring pattern filtering over files or standard input.
    - `df`: VFS disk usage, block sizes, and mount point statistics.
    - `dmesg`: Userland viewer for kernel circular diagnostic buffer.
    - `date`: Human-readable system uptime clock formatter.
    - `sync`: Explicit VFS buffer flush utility to persistent flash storage.
    - `nslookup`: RFC 1035 UDP DNS client resolving IPv4 A-records from nameservers.
    - `curl`: HTTP/1.1 client for streaming web endpoints over TCP sockets.
    - `mv`: File move and rename utility with chunked streaming and source unlinking.
    - `stat`: File and directory metadata inspector computing exact byte sizes.
    - `tee`: Stream splitter duplicating standard input to standard output and files.
    - `env`: Environment variable, working directory, and operating system release viewer.
    - `clear`: Terminal screen reset and cursor repositioning via ANSI escape sequence.
    - `pwd`: Current working directory query from process execution context.
    - `poweroff`: Clean hardware shutdown placing development board into deep sleep.
    - `mount`: Active VFS mount table and filesystem type inspector.
    - `umount`: Filesystem unmount and busy/system-managed status reporter.
    - `less`: Interactive screen-by-screen file and stream pager with navigation controls.
    - `find`: Non-recursive BFS directory hierarchy search with name pattern matching.
    - `du`: Recursive disk space measurement for directory trees and individual files.
    - `service`: System daemon and service manager coordinating system and userland unit lifecycles.
    - `pkg`: System package manager inspecting binary metadata, execution architectures, and repository integrity.
    - `which`: Locates executable binary paths in the system binary path.
    - `true`: Standard POSIX utility returning success exit code 0.
    - `false`: Standard POSIX utility returning failure exit code 1.
- **Phase 3: Security Hardening & SSH Engine (P1)**:
  - Hardware TRNG generation of Ed25519 host key on initial boot.
  - Persistence of host key seed in dedicated NVS flash storage (`ESHK` format with CRC checksum), guaranteeing stable host identity across reboots.
  - Standalone RFC 4648 Base64 encoding and decoding engine.
  - Passwordless public-key SSH login support for `ssh-ed25519` keys via authorized keys file with constant-time verification.
  - SSH connection timeouts: 60-second authentication deadline and 300-second session idle timeout.
  - Clean TCP socket teardown on remote FIN/RST (`CloseWait`), immediately releasing shell sessions and channels.
  - Implemented `/dev/random` and `/dev/urandom` character devices streaming cryptographically secure random entropy directly from the on-chip True Random Number Generator.
- **Phase 4 & 5: Bus Drivers, BLE & OTA Resilience (P2)**:
  - Verified I2C and full-duplex SPI master drivers with userland ioctl memory protection.
  - Non-blocking BLE advertising integrated directly into the network task service loop.
  - Fixed OTA state machine image validation logic (correctly transitioning candidate images to `VALID` state).
  - Enhanced firmware image header verification with minimum size and segment bounds checks.
  - Bootloader slot and candidate verification state detection and notice printed during startup.
- **Phase 6: VFS Hierarchy, Dynamic Mounts & Kernel Observability (P2/P3)**:
  - Dynamic active mount table tracking in VFS (`/proc/mounts`) mapping devices, mount points, and filesystem types.
  - Mounted secondary in-memory scratch volume on `/mnt` alongside root and temporary mounts.
  - Added CPU architecture and dual-core topology metrics export (`/proc/cpuinfo`).
  - Added diagnostic circular ring buffer with microsecond timestamps and system version identifiers in ProcFs.
  - Boot-time creation of full standard Unix directory tree (`/sbin`, `/var`, `/var/log`, `/var/lib`, `/home`, `/mnt`) alongside root and temporary mounts.
- **Phase 7: Advanced Capabilities, Feature Gates & Test Suite Expansion (P3)**:
  - Verified clean release compilation with dual-core symmetric multiprocessing (SMP) and memory protection unit (PMS) feature gates.
  - Extended automated test suite to 135 tests passing with 100% success rate across DNS, crypto, VFS logic, and file search algorithms.
- **Phase 8: Hardware Watchdog, GPIO Subsystem, LittleFS Engine & Core Unix Expansion (P1/P2)**:
  - Implemented hardware Watchdog Timer driver with automatic boot reset reason decoding from on-chip ROM, distinguishing cold power-on, software reboot, deep sleep wakeup, RTC watchdog, timer group watchdogs, and brownout detector events.
  - Added dedicated character device for the watchdog supporting single-byte ping writes, structured status inspection on read, and control ioctls for feeding, status checking, and timeout configuration.
  - Integrated periodic background watchdog feeding into the kernel heartbeat task to continuously monitor scheduler liveness and guard against system hangs.
  - Exposed live watchdog metrics and boot reset reason diagnostics in the virtual process filesystem.
  - Implemented dedicated GPIO character device providing userland control over pin direction configuration, digital level writes, pin reads, and output toggling for pins 0 through 48.
  - Implemented a resilient LittleFS hierarchical filesystem engine featuring inode abstractions, subdirectories, byte-level file reads and writes, directory traversal, file truncation, and unlinking.
  - Expanded userland application suite to **68 dynamic ELF binaries**, linking watchdog administration (`wdt`), direct GPIO manipulation (`gpio`), Unix network interface reporter (`ifconfig`), standard system information tool (`uname`), and host name manager (`hostname`).
  - Initialized system host name configuration and startup profile in the persistent configuration hierarchy.
  - Extended the automated test suite to 142 tests passing with 100% success rate across watchdog logic, GPIO pin bounds, filesystem geometry, and system utilities.
- **Phase 9: Real-time Performance Monitoring, Stream Processing & Extended POSIX Suite (P1/P2)**:
  - Expanded userland application suite to **74 dynamic ELF binaries**, all operating within their 16 KB PSRAM execution slot budgets.
  - Implemented real-time system performance and task observer (`top`) rendering CPU clock frequencies, memory allocations, and task execution stack metrics with ANSI screen refresh.
  - Implemented user identity inspector (`whoami`) reporting current session username.
  - Implemented stream field extractor (`cut`) supporting custom delimiters and field column selection across files and pipelines.
  - Implemented stream character translator and deleter (`tr`) for translating byte ranges and stripping unwanted carriage returns from terminal streams.
  - Implemented standard Base64 encoding and decoding utility (`base64`) compliant with RFC 4648 for stream and file processing.
  - Implemented canonical hexadecimal and ASCII byte dumper (`hexdump`) for binary inspection of filesystem data and hardware character devices.
  - Integrated system startup script execution with environment profile sourcing.
  - Expanded the automated test suite to 146 tests passing with 100% success rate.
- **Phase 10: Hardware PWM & SAR ADC Drivers, Kernel ProcFs Introspection & Core System Expansion (P1/P2/P3)**:
  - Implemented LEDC hardware Pulse Width Modulation (PWM) driver with 8 independent channels (0 through 7), configurable timer frequencies (1 Hz to 40 MHz) with fractional 10.8 fixed-point divider calculation, 10-bit duty cycle resolution, and dynamic GPIO matrix pin routing for pins 0 through 48.
  - Added dedicated PWM character device supporting tabular channel status dumps on read, text commands (`config`, `duty`, `disable`) on write, and binary ioctl interfaces for channel configuration and duty cycle modulation.
  - Implemented SAR Analog-to-Digital Converter (ADC) driver for 10 analog channels (channels 0 through 9 corresponding to physical pins 1 through 10) with 12-bit hardware resolution, calibrated millivolt conversion (0 to 3300 mV), sample averaging, and fallback entropy smoothing.
  - Added dedicated ADC character device supporting formatted multichannel voltage readouts with ASCII visual bar gauges, text reading commands on write, and binary ioctls for raw conversion and millivolt queries.
  - Expanded virtual process filesystem introspection with registered character and block device driver table (`/proc/devices`), CPU interrupt lines, vectors, and scheduler tick counters (`/proc/interrupts`), flash partition layout and sector counts (`/proc/partitions`), and hardware cryptography engine status (`/proc/crypto`).
  - Added system power configuration node in the system filesystem (`/sys/power`) exposing CPU clock speed, APB clock speed, crystal frequency, and sleep mode capabilities.
  - Expanded userland application suite to **80 dynamic ELF binaries**, all linking within their 16 KB PSRAM execution slot budgets:
    - `adc`: Real-time ADC reader reporting channel voltages, raw 12-bit values, and graphical level gauges.
    - `pwm`: Multi-channel PWM configuration and duty cycle modulation utility.
    - `cal`: Standard Unix calendar utility printing formatted month grids and calculating day-of-week alignment.
    - `basename`: Strips directory paths and optional file suffixes.
    - `dirname`: Extracts parent directory paths from file path strings.
    - `lsof`: Lists open files, active file descriptors, network sockets, and mounted filesystems across the system.
  - Expanded the automated test suite to **157 tests passing with 100% success rate** across PWM clock calculations, ADC millivolt conversions, process filesystem structures, and Unix utility algorithms.
- **Phase 11: Inter-Process Communication Named Pipes (FIFOs), Stream Manipulation & Full System Utilities (P1/P2/P3)**:
  - Implemented named pipe (FIFO) inter-process communication within the virtual filesystem, supporting bounded in-memory byte queues with blocking and non-blocking read/write semantics across process boundaries.
  - Integrated named pipe creation into the filesystem syscall interface, preserving binary ABI backward compatibility via special mode bit flag detection.
  - Added userland C library binding and system utility for creating named pipes in file hierarchies.
  - Implemented byte-by-byte file comparison utility (`cmp`) identifying divergence offsets and line numbers.
  - Implemented in-place stream sorting utility (`sort`) supporting alphabetic order, reverse sorting, and numerical comparison.
  - Implemented duplicate line filter and reporter (`uniq`) supporting count prefixing and duplicate-only extraction.
  - Implemented system log generator (`logger`) appending structured syslog entries with facility and severity levels to persistent logs.
  - Implemented command construction utility (`xargs`) converting standard input streams into argument vectors for command batching.
  - Expanded userland application suite to **86 dynamic ELF binaries**, all linking within their 16 KB PSRAM execution slot budgets.
  - Expanded automated test suite to **165 tests passing with 100% success rate** covering FIFO ring buffers, syscall mode detection, stream sorting, duplicate filtering, byte-by-byte comparison, and command batching.
- **Phase 12: Scheduled Daemon Services, System Resource Introspection & POSIX Tooling (P1/P2/P3)**:
  - Implemented scheduled cron daemon service managing periodic system tasks and background maintenance routines.
  - Added system load average, supported filesystem registry, and active IPC subsystem nodes to the virtual process filesystem.
  - Upgraded system calendar and date calculation to convert Unix epoch timestamps into UTC Gregorian calendar dates via Howard Hinnant's algorithm.
  - Implemented periodic command execution observer (`watch`) with ANSI screen clear and configurable cycle intervals.
  - Implemented printable character sequence scanner (`strings`) for binary files and memory streams.
  - Implemented temporary file and directory creator (`mktemp`) with pseudo-random entropy generation.
  - Implemented Inter-Process Communication facility reporter (`ipcs`) inspecting active named pipes, queues, and network endpoints.
  - Implemented command execution timeout supervisor (`timeout`) enforcing time limits and process termination.
  - Expanded userland application suite to **92 dynamic ELF binaries**, all linking within their 16 KB PSRAM execution slot budgets.
  - Expanded automated test suite to **171 tests passing with 100% success rate** covering Gregorian date math, crontab parsing, mktemp entropy suffixes, strings extraction, and timeout status codes.
- **Phase 13: Interactive Text Editor, Archival Tooling & POSIX User Administration (P1/P2/P3)**:
  - Implemented interactive Unix line editor (`ed`) supporting line-by-line file inspection, text buffer appending, deletion, and persistent storage saving.
  - Implemented user and group identity reporter (`id`) resolving real and effective numerical and symbolic user and group identifiers.
  - Implemented session user switching utility (`su`) allowing seamless invocation of interactive shells with specified user credentials.
  - Implemented standard USTAR tape archiver (`tar`) supporting multi-file archive creation, table of contents inspection, and byte-precise file extraction.
  - Implemented virtual memory and system activity observer (`vmstat`) reporting thread scheduling frequencies, memory allocations, and tick rates.
  - Implemented line-by-line file difference comparator (`diff`) detecting divergences with unified difference indicators.
  - Expanded userland application suite to **98 dynamic ELF binaries**, all linking within their 16 KB PSRAM execution slot budgets.
  - Expanded automated test suite to **177 tests passing with 100% success rate** covering USTAR block layouts, diff comparisons, line editor buffers, and user identity mappings.
- **Phase 14: Virtual Framebuffer Graphics, POSIX Scripting Expressions & System Administration (P1/P2/P3)**:
  - Implemented virtual monochrome framebuffer display driver supporting screen geometry inspection, clearing, filling, inverting, and direct byte rendering via standard device ioctls.
  - Added network neighbor ARP cache introspection to the virtual process filesystem.
  - Implemented standard POSIX conditional evaluator supporting file existence tests, string length checks, and relational comparisons for shell scripting.
  - Implemented mathematical and string expression evaluator computing arithmetic operations, modular arithmetic, and comparisons.
  - Implemented unified diff patch utility reading unified diff files and applying hunk modifications directly to target files.
  - Implemented network neighbor cache inspection utility displaying IP-to-MAC associations and interface bindings.
  - Implemented system log rotation utility monitoring log file sizes and archiving logs upon exceeding predefined thresholds.
  - Implemented framebuffer graphics diagnostic utility validating screen dimensions, drawing test patterns, inverting pixels, and verifying display buffers.
  - Expanded userland application suite to **104 dynamic ELF binaries**, all linking within their 16 KB PSRAM execution slot budgets.
  - Expanded automated test suite to **183 tests passing with 100% success rate** covering framebuffer geometry calculations, arithmetic expression evaluation, POSIX conditional operators, diff patching, log rotation thresholds, and ARP table parsing.
- **Phase 15: Network Socket Plumbing, Terminal Introspection & Core Data Processing Tools (P1/P2/P3)**:
  - Added network device traffic statistics and routing table entries to the virtual process filesystem.
  - Implemented arbitrary TCP network connection and listener tool (`nc`) supporting client socket connections, server port listening, and bidirectional data streaming.
  - Implemented environment variable introspection utility (`printenv`) querying active session variables or displaying specific values.
  - Implemented sorted file comparison utility (`comm`) outputting unique and shared lines across three text columns with line suppression options.
  - Implemented line numbering utility (`nl`) formatting input streams with right-aligned line numbers and customizable padding.
  - Implemented text folding utility (`fold`) wrapping long lines to fixed column widths with whitespace-aware word boundaries.
  - Implemented file splitting utility (`split`) dividing large files into byte-size or line-count segments with alphabetic suffixes.
  - Expanded userland application suite to **110 dynamic ELF binaries**, all linking within their 16 KB PSRAM execution slot budgets.
  - Expanded automated test suite to **189 tests passing with 100% success rate** covering sorted file column generation, line numbering padding, text folding, chunk suffix calculation, environment lookups, and network device metrics.
- **Phase 16: Kernel Boot Argument Introspection & Classic Unix Processing Tools (P1/P2/P3)**:
  - Added kernel command-line and boot argument introspection (`/proc/cmdline`) to the virtual process filesystem.
  - Implemented continuous string generation utility (`yes`) supporting bounded repetitions and default affirmation streaming.
  - Implemented character-wise line reversal utility (`rev`) processing file inputs and standard streams.
  - Implemented multi-file line merging tool (`paste`) supporting custom column delimiters and parallel line interleaving.
  - Implemented tab-to-space expansion utility (`expand`) replacing horizontal tab characters with column-aligned spaces.
  - Implemented space-to-tab compression utility (`unexpand`) reducing consecutive column spaces into tab stops.
  - Implemented prime factorization utility (`factor`) decomposing large integer inputs into prime factors using trial division and 6k±1 optimization.
  - Expanded userland application suite to **116 dynamic ELF binaries**, all linking within their 16 KB PSRAM execution slot budgets.
  - Expanded automated test suite to **194 tests passing with 100% success rate** covering prime factor decomposition, string reversal, multi-file line merging, tab expansion, and boot command-line introspection.
- **Phase 17: System Statistics Introspection, POSIX Integrity & Extended Command Toolchain (P1/P2/P3)**:
  - Added system-wide CPU tick counters, context switch tallies, process counters, and boot timestamps (`/proc/stat`) to the virtual process filesystem.
  - Implemented sequence generator utility (`seq`) supporting positive/negative increments, customizable separators, and equalized zero-padding.
  - Implemented hangup-immune process execution utility (`nohup`) with automatic output redirection to non-volatile terminal logs.
  - Implemented random permutation and shuffling tool (`shuf`) with Fisher-Yates entropy scrambling over input files, command arguments, and numeric ranges.
  - Implemented classic octal/hex/decimal dump utility (`od`) formatting byte sequences with octal offsets and character escape visualization.
  - Implemented absolute path canonicalization tool (`realpath`) resolving relative directory segments, parent traversal, and current working directory prefixes.
  - Implemented POSIX standard 32-bit CRC checksum and byte count utility (`cksum`) compliant with IEEE Std 1003.1.
  - Expanded userland application suite to **122 dynamic ELF binaries**, all linking within their 16 KB PSRAM execution slot budgets.
  - Resolved EspFs compaction log size expansion by skipping all-zero chunks to preserve sparse holes and recording logical file sizes with lightweight truncation markers.
  - Resolved non-reentrant kernel mutex hangs by implementing core-aware owner tracking and immediate panic diagnostics on same-core re-entry.
  - Resolved CPU light sleep and idle power consumption by implementing hardware wait-for-interrupt low-power sleep in the scheduler idle task and power control ioctl, replacing hot spin loops without bus hangs.
  - Expanded automated test suite to **198 tests passing with 100% success rate** covering sequence generation, path canonicalization, POSIX checksum vectors, and CPU statistics introspection.
- **Phase 18: System Configuration Plumbing, Desk Calculator & Core POSIX Expansion (P1/P2/P3)**:
  - Initialized standard Unix configuration hierarchy on initial boot with persistent host lookup tables, domain name resolution records, operating system release metadata, issue banners, and user group definitions.
  - Implemented static hostname resolution in network diagnostic utilities, resolving local loopback and device hostnames directly from system host configuration without requiring external domain name queries.
  - Added dynamic nameserver extraction from system resolver configuration for domain name lookups.
  - Implemented execution duration profiler and timer tool (`time`) measuring real wall-clock elapsed execution time and tracking child process exit status codes.
  - Implemented symbolic target and path canonicalization utility (`readlink`) resolving parent directory traversal and suppressing trailing delimiters.
  - Implemented standard terminal identification tool (`tty`) detecting character device streams versus redirected regular file descriptors on standard input.
  - Implemented user group membership query tool (`groups`) resolving primary and supplementary group affiliations from the system group configuration.
  - Implemented numeric host identifier utility (`hostid`) computing a stable 32-bit hexadecimal identifier from system hostname parameters.
  - Implemented classic Unix Desk Calculator (`dc`) supporting reverse-Polish notation stack arithmetic, stack duplication, reversal, and stack dump inspection.
  - Expanded userland application suite to **128 dynamic ELF binaries**, all linking within their 16 KB PSRAM execution slot budgets.
  - Expanded automated test suite to **204 tests passing with 100% success rate** covering static host resolution, resolver parsing, reverse-Polish stack evaluation, host identification hashing, group membership matching, and execution duration formatting.
- **Phase 19: Kernel Sysctl Introspection, Infix Arithmetic & Standard Process Execution (P1/P2/P3)**:
  - Implemented virtual system control introspection nodes in the virtual process filesystem exposing kernel hostnames, OS release identifiers, OS types, build versions, and memory statistics with dynamic write support for live hostname updates.
  - Implemented standard Unix system control administration utility (`sysctl`) supporting bulk variable enumeration, specific variable querying, and dynamic runtime parameter configuration.
  - Implemented available processor count reporting utility (`nproc`) distinguishing active symmetric multiprocessing cores from uniprocessor execution and supporting hardware processor query flags.
  - Implemented user login identity inspection utility (`logname`) querying authenticated session usernames from system configuration.
  - Implemented root directory jail execution utility (`chroot`) changing process execution roots and launching subshells or batch commands within specified directory environments.
  - Implemented standard pagination and print formatting utility (`pr`) formatting text streams with title banners, page counts, customizable page lengths, and trailer margins.
  - Implemented classic Unix infix arithmetic calculator (`bc`) supporting standard mathematical expressions with parentheses, operator precedence, modulo, and exponentiation using Dijkstra's Shunting-Yard evaluation algorithm.
  - Expanded userland application suite to **134 dynamic ELF binaries**, all linking within their 16 KB PSRAM execution slot budgets.
  - Expanded automated test suite to **209 tests passing with 100% success rate** covering system parameter paths, infix arithmetic precedence, pagination page geometries, processor core parsing, and command path resolution.
- **Phase 20: System Logging Infrastructure, Topological Sorting, Stream Formatting & Text Processing (P1/P2/P3)**:
  - Initialized persistent system logging hierarchy with automatic creation of persistent system log journals and kernel ring buffer message persistence on boot.
  - Enhanced kernel ring buffer viewer (`dmesg`) supporting kernel memory ring introspection, persistent system log interrogation, custom log file reading, and command-line usage guidance.
  - Implemented classic Unix paragraph reflow and text formatting utility (`fmt`) supporting customizable line widths, word wrapping, paragraph preservation, and split-only formatting mode.
  - Implemented POSIX topological sorting utility (`tsort`) resolving directed acyclic graph partial orderings, dependency pipelines, and detecting dependency cycles.
  - Implemented context-based file splitting utility (`csplit`) dividing input streams by line number offsets or regular pattern context boundaries with configurable chunk prefix naming and byte-count telemetry.
  - Implemented process execution priority utility (`nice`) querying default scheduling niceness and executing child processes with priority adjustments.
  - Implemented DOS-to-Unix line termination conversion utility (`dos2unix`) stripping carriage return characters from CRLF streams and files with in-place atomic replacement.
  - Implemented Unix-to-DOS line termination conversion utility (`unix2dos`) converting LF stream delimiters to CRLF sequences with in-place atomic file modification.
  - Expanded userland application suite to **140 dynamic ELF binaries**, all linking within their 16 KB PSRAM execution slot budgets.
  - Expanded automated test suite to **214 tests passing with 100% success rate** covering paragraph reflow geometries, topological ordering, context splitting, line break conversions, and persistent logging paths.

### [0.1.0] - 2026-08-06

#### Added
- Initial release of EspressoOS monolithic `no_std` kernel in Rust for ESP32-S3.
- Preemptive multitasking driven by SYSTIMER at 100 Hz.
- 8 MB Octal PSRAM support with 1 MB reserved for PSRAM execution pool (32 slots of 16 KB).
- Dynamic ELF binary loader with literal fixup table relocations.
- Persistent EspFs log-structured filesystem on NOR flash.
- Wi-Fi Station (STA) driver with `smoltcp` IPv4 TCP/UDP stack and DHCP client.
- SSH-2.0 daemon supporting curve25519-sha256, ssh-ed25519, and chacha20-poly1305@openssh.com.
- Interactive `/bin/sh` shell with multi-stage pipelines and redirection.
- Initial suite of 35 userland binaries in `/bin/*`.

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
