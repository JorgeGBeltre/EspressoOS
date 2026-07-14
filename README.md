# EspressoOS — A `no_std` Unix-like Operating System in Rust for ESP32-S3

[![Rust Version](https://img.shields.io/badge/Rust-1.75%2B%20%2F%20Xtensa-orange?logo=rust)](https://github.com/esp-rs/rust)
[![Target Platform](https://img.shields.io/badge/Platform-ESP32--S3--WROOM--1-blue?logo=espressif)](https://www.espressif.com/en/products/socs/esp32-s3)
[![License](https://img.shields.io/badge/License-MIT%20or%20Apache--2.0-teal)](LICENSE)
[![Status](https://img.shields.io/badge/Status-Boots%20%2B%20SSH%20login%20on%20hardware-brightgreen)](#current-status-running-on-hardware)

---

**EspressoOS** is a Unix-like operating system written entirely from scratch in `no_std` Rust for the **ESP32-S3-WROOM-1-N16R8** development board (Xtensa LX7 dual-core, 16 MB flash, and 8 MB PSRAM). 

The system implements fundamental operating system concepts (a Virtual File System "VFS" with an everything-is-a-file abstraction, a cooperative/preemptive multitasking scheduler with a hand-written Xtensa context switch, kernel-level peripheral drivers, a stable system call "syscall" ABI, a WiFi + TCP/IP network stack, an SSH-2.0 server, and an interactive REPL shell) in a resource-constrained embedded hardware environment.

---

## Current Status (Running on Hardware)

EspressoOS **boots and runs on a physical ESP32-S3** and is reachable over the network via SSH. The following is verified working on real silicon:

| Capability | Status |
| :--- | :--- |
| Compiles & links for `xtensa-esp32s3-none-elf` | ✅ |
| Boots: HAL init, kernel heap, VFS (`ramfs` on `/` and `/tmp`, `devfs` on `/dev`) | ✅ |
| **Multitasking** — hand-written Xtensa windowed-register context switch | ✅ cooperative, stable |
| **8 MB PSRAM** mapped and added to the kernel heap | ✅ |
| **WiFi (STA) + DHCP + TCP/IP** (`esp-wifi` + `smoltcp`) | ✅ (obtains an IP, TCP echo on port 2323) |
| **SSH-2.0 server** (curve25519-sha256, ssh-ed25519, chacha20-poly1305@openssh) | ✅ `ssh <user>@<ip>` opens the shell |
| Interactive shell over SSH (`ls`, `cat`, `mkdir`, `cd`, `pwd`, `write`, redirections…) | ✅ |

> **Log in with a standard client:** `ssh youareme@<board-ip>` (dev credentials in [`ssh/config.rs`](kernel/src/drivers/ssh/config.rs); see [Connecting via SSH](#3-connect-over-ssh)).

**Implemented since (compile-clean, pending on-device checks):** persistent storage (`EspFs` at `/`, survives reboot), I2C/SPI bus drivers (`/dev/i2c0` · `/dev/spi0`), the **syscall ABI** (+ a real `syscall`-instruction trap under `--features syscall-trap`), an **OTA image receiver** (TCP :3300 → `ota apply`), and — behind opt-in features — **PMS** memory protection enforced at boot (`--features pms`) and **real cross-core scheduling** on the APP_CPU (`--features smp`). Every feature combination builds and links in `--release`; **104/104** logic tests pass. **Still open:** safe **preemptive** scheduling (`need_resched` in the vector epilogue), verifying OTA boot-switch against the stock bootloader, and shared-queue cross-core migration. See the [roadmap](#project-status-and-development-roadmap).

---

## Table of Contents

- [Project Status and Development Roadmap](#project-status-and-development-roadmap)
- [Key Features](#key-features)
- [Repository Structure](#repository-structure)
- [Environment Prerequisites](#environment-prerequisites)
- [Building and Flashing](#building-and-flashing)
- [Memory Map and Partition Table](#memory-map-and-partition-table)
- [Kernel Subsystems](#kernel-subsystems)
  - [VFS (Virtual File System)](#vfs-virtual-file-system)
  - [Task Scheduler](#task-scheduler)
  - [Syscall Mechanism (System Calls)](#syscall-mechanism-system-calls)
  - [A/B Update Scheme (OTA)](#ab-update-scheme-ota)
  - [Peripheral Device Drivers](#peripheral-device-drivers)
  - [Networking (WiFi + TCP/IP)](#networking-wifi--tcpip)
  - [SSH Server (Working)](#ssh-server-working)
- [Interactive REPL Shell](#interactive-repl-shell)
- [Verification and Mock Testing](#verification-and-mock-testing)
- [Contribution and License](#contribution-and-license)
- [License](#license)
- [Contact](#contact)
- [Support](#support)

---

## Project Status and Development Roadmap

EspressoOS development is structured into **10 incremental phases**. Bring-up (P0), memory/PSRAM (P1), multitasking (P2, cooperative), and networking with an SSH server (P7) are **verified on hardware**. Bus drivers (P3), persistent storage (P4, `EspFs`), and the syscall ABI (P6) are **wired and compile-clean** (pending on-device checks). OTA (P5) is partially wired; memory protection (P8) and SMP (P9) are implemented **behind opt-in cargo features** so the default image stays the known-good one. Progress is not strictly sequential — SSH was brought up early because the scheduler and network stack were ready.

```
Phase 0 ──────► Phase 1 ──────► Phase 2 ──────► Phase 3 ──────► Phase 4 
Bring-up       Memory         Multitasking   Bus Drivers    Storage & VFS
(✅ done)       (✅ PSRAM)      (✅ sched.)     (✅ I2C/SPI)    (✅ EspFs)
  │
  ├──────────► Phase 5 ──────► Phase 6 ──────► Phase 7 ──────► Phase 8 ──────► Phase 9
               OTA A/B        Syscalls/      Networking     Memory         SMP Dual-Core
               (✅ OTA rx)     (✅ ABI)        (✅ WiFi+SSH)   (feat: pms)     (feat: smp)
```

### Development Phases Roadmap Table

| Phase | Title | Involved Components / Status |
| :--- | :--- | :--- |
| **Phase 0** | **Bring-up** | **DONE (on hardware).** Xtensa CPU clock setup, kernel heap (SRAM), console over UART0, VFS boot (mounting `/dev`, `/`, `/tmp`), scheduler with `idle` thread, interactive `shell`, and a `heartbeat` task. Requires `build-std` + the `-Tlinkall.x` linker script (see [Building](#building-and-flashing)). |
| **Phase 1** | **Memory Management (PSRAM)** | **DONE.** The 8 MB external octal PSRAM is mapped in `esp_hal::init` and registered as a secondary `esp-alloc` heap region ([mm/heap.rs](kernel/src/mm/heap.rs)). |
| **Phase 2** | **Task Scheduler** | **DONE (cooperative, on hardware).** Round-robin scheduling ([policy.rs](kernel/src/scheduler/policy.rs)) over a hand-written Xtensa windowed-register context switch ([context.rs](kernel/src/arch/xtensa/context.rs), FreeRTOS/esp-idf-style `XT_STK` frame + `rfe`/`retw`). **Pending:** safe preemption (`need_resched` in the vector epilogue) instead of switching inside the SYSTIMER ISR. |
| **Phase 3** | **Bus Drivers** | **Wired (compiles; needs a device to verify on hardware).** Master I2C ([i2c.rs](kernel/src/drivers/i2c.rs)) and SPI ([spi.rs](kernel/src/drivers/spi.rs)) drivers now receive their peripherals from `main` (no `Peripherals::steal()`), are exposed as `/dev/i2c0` and `/dev/spi0`, and driven by the `i2c` / `spi` shell commands (`i2c scan`, `spi transfer …`). |
| **Phase 4** | **Storage & Filesystems** | **Wired (compiles + logic-tested; verify persistence on hardware).** `EspFs` ([fs/espfs](kernel/src/fs/espfs/mod.rs)) — a pure-Rust **log-structured, wear-leveled** filesystem over the internal NOR flash ([flash.rs](kernel/src/drivers/flash.rs)) — is mounted at `/` (RamFs fallback), so files survive reboots. Chosen over the C `littlefs2` (kept as a stub) to avoid a bindgen/cc build on this `no_std` Xtensa target. 11 logic tests in `tools/tests/espfs_tests.py`. |
| **Phase 5** | **OTA A/B Updates** | **Wired.** An OTA image receiver on TCP **port 3300** ([drivers/wifi.rs](kernel/src/drivers/wifi.rs)) buffers the firmware into PSRAM (safe: no flash writes during the WiFi transfer); `ota apply` then flashes the inactive slot and updates `otadata` ([ota](kernel/src/ota/mod.rs)). `ota status`/`set`/`rx`/`apply` shell commands. **Boot-switching** relies on the ESP-IDF 2nd-stage bootloader `espflash` writes (it reads `otadata`) — verify on hardware; a custom bootloader is only needed if the stock one doesn't honor it. |
| **Phase 6** | **Syscalls & Userland** | **Wired.** `syscall::invoke` + the `syscalltest` command exercise the full ABI (arg marshalling → `dispatch` → errno). A real Xtensa `syscall`-instruction trap (EXCCAUSE=1, overriding the weak `__exception` and delegating other causes to esp-backtrace) is available behind `--features syscall-trap` ([syscall/trap.rs](kernel/src/syscall/trap.rs)). No privilege rings yet, so it is a mechanism, not isolation. |
| **Phase 7** | **Networking (WiFi) + SSH** | **DONE (on hardware).** 802.11 STA radio via `esp-wifi` bound to the `smoltcp` TCP/IP stack ([drivers/wifi.rs](kernel/src/drivers/wifi.rs)); DHCP-assigned IP; a **working SSH-2.0 server** on port 22 ([drivers/ssh](kernel/src/drivers/ssh)) serving the shell. |
| **Phase 8** | **Memory Protection (PMS)** | **Behind `--features pms`, enforced at boot.** [mm/mpu.rs](kernel/src/mm/mpu.rs) enables the ESP32-S3 DRAM0 PMS **violation monitor** and applies **World-1 SRAM enforcement** at boot (safe: the kernel runs in **World-0**), confining a future userland; `pms` / `pms world1` inspect and re-apply. The exact permission-field encoding still needs TRM validation on hardware. |
| **Phase 9** | **SMP Dual-Core** | **Behind `--features smp`, real cross-core scheduling.** [scheduler/core_sync.rs](kernel/src/scheduler/core_sync.rs) starts the **APP_CPU (core 1)** via `esp_hal::CpuControl`; core 1 runs the scheduler over its **own run-queue** (`ready1`) — `spawn_core1` places tasks there while WiFi stays on core 0 (esp-wifi affinity). A demo `core1-worker` proves execution on core 1 (`smp` command / `core1` serial heartbeats). Shared-queue migration with per-core preemption is the next hardware step. |

---

## Key Features

- **Pure Rust `no_std` Development**: Zero dependency on the C-based ESP-IDF framework or standard library; complete low-level control of the hardware.
- **Virtual File System (VFS)**: Uniform abstractions for devices and file operations (`/dev/console`, `/dev/null`, `/dev/zero`, `/tmp`). Standardized file descriptors (`Fd`) supporting `open`/`close`/`read`/`write`/`seek`/`readdir`.
- **Task Scheduler**: Round-robin multitasking with a **hand-written Xtensa windowed-register context switch** in assembly ([context.rs](kernel/src/arch/xtensa/context.rs)) — verified stable on hardware in **cooperative** mode (`yield_now()`). A SYSTIMER-driven preemptive path exists; making preemption fully safe (switching in the interrupt-vector epilogue rather than inside the ISR) is in progress.
- **System Call ABI**: POSIX-like system calls utilizing registers (e.g., `a2` for syscall index) to transition from user mode to kernel mode safely.
- **OTA Updates & Partition Mappings**: Core logic and structures compatible with the Espressif partition table format, prepared for safe, atomic OTA updates.
- **Custom Partition Generator**: Bundled Python utility to generate and inspect bin tables compatible with the hardware ROM bootloader.
- **Logical Verification Simulator**: A comprehensive Python-based mock test harness ([logic_tests.py](tools/tests/logic_tests.py)) that mimics the exact behavior of key OS modules for verification in targetless host environments.

---

## Repository Structure

The workspace is organized as follows:

```
EspressoOS/
├── .cargo/                 # Target configuration for Xtensa cross-compilation
├── bootloader/             # Second-stage custom bootloader (stub crate, Phase 5+)
│   ├── src/
│   │   ├── flash.rs        # Direct flash reading methods for boot
│   │   ├── multiboot2.rs   # Multiboot headers mapping
│   │   ├── partition_table.rs # Basic partition table parser
│   │   └── main.rs         # Low-level boot initialization and kernel jump
│   └── Cargo.toml
├── kernel/                 # Core EspressoOS Kernel Crate
│   ├── src/
│   │   ├── arch/           # Architecture Specific Abstractions (Xtensa LX7)
│   │   │   ├── xtensa/
│   │   │   │   ├── context.rs    # Context saving/restoring and switch_to (Assembly)
│   │   │   │   ├── interrupts.rs # Vector table configuration & critical sections
│   │   │   │   ├── sync.rs       # Atomic synchronization (SMP-ready Mutex)
│   │   │   │   ├── timer.rs      # Driver for SYSTIMER (systick counting)
│   │   │   │   └── mod.rs
│   │   │   └── mod.rs
│   │   ├── drivers/        # Hardware Device Drivers
│   │   │   ├── gpio.rs     # GPIO configuration (Input/Output/Pull-up/Pull-down)
│   │   │   ├── uart.rs     # Console serial driver using native USB-Serial-JTAG
│   │   │   ├── flash.rs    # NOR flash driver via ROM SPI routines
│   │   │   ├── spi.rs      # SPI master bus driver (skeleton)
│   │   │   ├── i2c.rs      # I2C master bus driver (skeleton)
│   │   │   ├── wifi.rs     # Network task: esp-wifi STA + smoltcp + DHCP + echo/SSH (working)
│   │   │   ├── ssh/        # Working SSH-2.0 server (serves the shell on port 22)
│   │   │   │   ├── auth.rs        # User authentication (password / publickey ed25519)
│   │   │   │   ├── channel.rs     # Session channels & shell routing logic
│   │   │   │   ├── crypt.rs       # Session AEAD (chacha20-poly1305@openssh.com)
│   │   │   │   ├── kex.rs         # Key exchange (curve25519-sha256 ECDH + KDF, ed25519 host key)
│   │   │   │   ├── proto.rs       # RFC 4251 types and Binary Packet Protocol
│   │   │   │   ├── crypto_rng.rs  # Hardware TRNG -> rand_core adapter
│   │   │   │   ├── config.rs      # Dev credentials, authorized keys, host-key seed
│   │   │   │   └── mod.rs         # Connection state machine and server entry
│   │   │   └── mod.rs
│   │   ├── fs/             # Core File System Drivers
│   │   │   ├── ramfs.rs    # In-memory RAM filesystem
│   │   │   ├── littlefs/   # LittleFS integration layer (draft wrapper)
│   │   │   └── mod.rs
│   │   ├── mm/             # Memory Management Units
│   │   │   ├── heap.rs     # Kernel static allocator configuration (esp-alloc)
│   │   │   ├── mpu.rs      # Memory protection interface (PMS / World Controller)
│   │   │   └── mod.rs
│   │   ├── ota/            # Firmware updates and slot rotation logic
│   │   │   ├── partition.rs # CRC32 checker and Espressif otadata partition selector
│   │   │   └── mod.rs
│   │   ├── scheduler/      # CPU Task Scheduling
│   │   │   ├── task.rs     # Task Control Block (TCB), stacks and task lifecycle states
│   │   │   ├── policy.rs   # Round-Robin scheduling policy implementations
│   │   │   ├── core_sync.rs # Multicore/SMP boot and sync routines (skeleton)
│   │   │   └── mod.rs
│   │   ├── shell/          # Interactive REPL Environment
│   │   │   ├── parser.rs   # CLI tokenizer and syntax parser (redirections, pipes)
│   │   │   ├── remote.rs   # Shell Io trait and local/remote (SSH) adapters
│   │   │   ├── commands/   # Shell built-in coreutils (echo, uptime, free, ps, etc.)
│   │   │   └── mod.rs
│   │   ├── syscall/        # Syscall Dispatcher
│   │   │   ├── table.rs    # Syscall numbers mapping table
│   │   │   ├── handler.rs  # Kernel-side syscall handlers
│   │   │   └── mod.rs
│   │   ├── vfs/            # Virtual File System Layers
│   │   │   ├── devfs.rs    # Device files mount provider (/dev/null, /dev/console)
│   │   │   ├── file.rs     # Open file descriptors and access flags
│   │   │   ├── inode.rs    # VFS traits for inodes and filesystems
│   │   │   ├── mount.rs    # VFS mount point database and path normalization
│   │   │   └── mod.rs
│   │   ├── prelude.rs      # Shared definitions (KError, KResult, layout constants)
│   │   └── main.rs         # Kernel boot sequencer and main entry point
│   └── Cargo.toml
├── tools/                  # Build tools and utilities
│   ├── partition-gen/      # CSV-to-Binary partition layout compiler
│   │   └── partition_gen.py
│   ├── mkimage/            # Packaging tools for firmware binary creation (skeleton)
│   │   └── README.md
│   └── tests/              # Test suites
│       ├── logic_tests.py  # Python emulation suite to test OS logic
│       ├── ssh_proto_tests.py # Test suite for SSH packet formatting & types
│       └── run_all.py         # Unified Python test runner
├── Cargo.toml              # Parent workspace Cargo configuration
├── partitions.csv          # Flash memory layout specification
├── rust-toolchain.toml     # Toolchain pin configuration
└── README.md               # This documentation file
```

---

## Environment Prerequisites

To compile and flash EspressoOS, you will need the Espressif Xtensa Rust toolchain, flashing tools, and Python 3.

### 1. Install Rust + Espressif Xtensa Toolchain & Flashing Tools

Install `espup` (the toolchain installer) and `espflash` (flashing and monitoring utility) using Cargo:

```bash
# Install toolchain and flashing utilities
cargo install espup --locked
cargo install espflash@3.3.0 --locked
```

> **Important — use espflash 3.x.** This project targets `esp-hal 0.23`, whose image format predates the **ESP-IDF App Descriptor** that `espflash` **4.x** requires. With espflash 4.x the flashed image is rejected by the bootloader (`Image requires efuse blk rev …` / `no bootable app`). Until the project migrates to `esp-hal 1.0` + `esp-bootloader-esp-idf`, pin espflash to the **3.x** line (`3.3.0`).

Once installed, run `espup` to set up the toolchain on your system:
```bash
espup install
```

### 2. Configure Environment Variables

Based on your operating system and shell, you must source the environment script to make the Xtensa toolchain accessible in your current terminal session:

#### On Windows (PowerShell)
You may need to adjust the execution policy in your session to run the script:
```powershell
# Allow local script execution (run as Administrator if needed, or CurrentUser)
Set-ExecutionPolicy RemoteSigned -Scope CurrentUser

# Load Espressif environment variables
. $HOME\export-esp.ps1
```

#### On Linux / macOS
```bash
source $HOME/export-esp.sh
```

To verify that the tools and environment are set up correctly, check the flashing tool version:
```bash
espflash --version
```

### 3. Hardware Connections
Connect your **ESP32-S3** board to the host PC over USB. The kernel console is configured for **UART0** (`esp-println` `uart` feature), which is what most dev boards expose through an on-board USB-to-UART bridge (CH343 / CP2102 / CH340). The serial port will appear as e.g. `COM5` (Windows) or `/dev/ttyUSB0` (Linux).

> If your board only exposes the **native USB-Serial-JTAG** peripheral instead of a UART bridge, switch the `esp-println` feature in [`kernel/Cargo.toml`](kernel/Cargo.toml) from `uart` to `jtag-serial` (exactly one of `uart`/`jtag-serial`/`auto` may be enabled).

Your board also needs a **2.4 GHz WiFi** network in range for the networking/SSH features (the ESP32-S3 radio is 2.4 GHz only).

---

## Building and Flashing

Make sure you are in the repository directory before building:
```bash
cd EspressoOS
```

### 1. Compile the Partition Table
The flash memory must be partitioned before compiling and flashing the operating system. Run the Python generator from the repository root:

```bash
python tools/partition-gen/partition_gen.py
```
This utility parses [partitions.csv](partitions.csv) and compiles it into `partitions.bin`, validating partition alignments and boundary limits.

### 2. Configure WiFi Credentials
The networking and SSH features need the credentials of a **2.4 GHz** network. Copy the template to a git-ignored file and fill in your SSID/password:

```bash
cp kernel/src/wifi_credentials.rs.example kernel/src/wifi_credentials.rs
```
```rust
// kernel/src/wifi_credentials.rs  — git-ignored, never committed
pub const WIFI_SSID: &str = "your-2.4GHz-ssid";
pub const WIFI_PASSWORD: &str = "your-password";
```
> `wifi_credentials.rs` is listed in `.gitignore`, so your password never lands in version control — only the `.example` template is tracked.

### 3. Build the Kernel
Compile the kernel binary optimized for the Xtensa architecture. **A release build is required** (PSRAM and esp-wifi do not work in debug):
```bash
cargo build --release
```

### 4. Flash and Monitor
Write the binary onto the ESP32-S3 flash and start the serial monitor with a single command:
```bash
cargo run --release
```
*(This command runs `espflash flash --monitor` automatically as configured in `.cargo/config.toml`)*

#### Expected Serial Output
```
[kernel] PSRAM added to heap: 8388608 bytes @ 0x3c000000
========================================
   EspressoOS   ·   kernel
========================================
[kernel] task 'shell' created (tid=1)
[kernel] task 'heartbeat' created (tid=2)
[net] connecting to SSID 'your-ssid'...
[net] associated; negotiating DHCP...
[net] IP = 192.168.2.126
[net] TCP echo server listening on port 2323
[ssh] SSH server on port 22 (try: ssh youareme@192.168.2.126)
[ssh] host key: SHA256:ODaZ7h4sUydeKsOw4lDsvi4bSji78Jrczj4kYWfTrS8
[heartbeat] tick=0 uptime=236ms led=1
[heartbeat] tick=1 uptime=736ms led=0
...
```
The on-board LED blinks (~1 Hz). If it doesn't, adjust the `LED_GPIO` constant in [main.rs](kernel/src/main.rs) to match your board (typically GPIO 2 or GPIO 48).

### 5. Connect over SSH
Once the board prints its IP and `SSH server on port 22`, log in from any standard OpenSSH client on the same network:

```bash
ssh youareme@192.168.2.126        # then enter the dev password
```

Development credentials live in [`kernel/src/drivers/ssh/config.rs`](kernel/src/drivers/ssh/config.rs) (`DEV_USER` / `DEV_PASSWORD`) — change them for your setup. The **host key** is derived from a **fixed dev seed** (`HOST_KEY_SEED`) so its fingerprint is **stable across reboots and re-flashes** — you won't need to clear `known_hosts` on every connection.

You can also verify the raw network path with the TCP echo server:
```bash
# sends "hello" and gets it back
printf 'hello\n' | nc 192.168.2.126 2323
```

> **Security note (dev-only):** the SSH password, the authorized-keys list, and the host-key seed are placeholders embedded in the binary for the MVP. In production these would come from persistent storage (LittleFS: hashed passwords, `authorized_keys`, a TRNG-generated host key). Do **not** expose this server to the internet.

---

## Memory Map and Partition Table

EspressoOS lays out the **16 Megabytes** (`0x1000000` bytes) external SPI Flash to fit the A/B update rotation scheme, complying with the **4 KB** erase sector sizes and **64 KB** app partition alignment required by Xtensa:

```
Flash Address Map (16 MB):
0x000000 ├────────────────────────────┤ 0x008000: Bootloader (2nd Stage, 32 KB)
0x008000 ├────────────────────────────┤ 0x008C00: Partition Table binary (3072 B)
0x009000 ├────────────────────────────┤ 0x00F000: NVS (Non-Volatile Storage, 24 KB)
0x00F000 ├────────────────────────────┤ 0x011000: otadata (A/B Boot control data, 8 KB)
0x020000 ├────────────────────────────┤ 0x420000: Slot A - factory app (Primary Kernel, 4 MB)
0x420000 ├────────────────────────────┤ 0x820000: Slot B - ota_0 app (Secondary Kernel, 4 MB)
0x820000 ├────────────────────────────┤ 0xFF0000: File System (fs) (LittleFS / devfs / ramfs, ~7.8 MB)
0xFF0000 ├────────────────────────────┤ 0x100000: coredump (Crash dumps, 64 KB)
```

The matching flash map layout constants are declared in [prelude.rs](kernel/src/prelude.rs#L83-L106):

| Name | Type | Subtype | Flash Offset | Size | Purpose |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `nvs` | `data` | `nvs` | `0x9000` | 24 KB (`0x6000`) | Non-Volatile key-value system config store. |
| `otadata` | `data` | `ota` | `0xF000` | 8 KB (`0x2000`) | Sequence state counters for A/B boot selection. |
| `factory` | `app` | `factory` | `0x20000` | 4 MB (`0x400000`) | Slot A: Primary boot kernel application. |
| `ota_0` | `app` | `ota_0` | `0x420000` | 4 MB (`0x400000`) | Slot B: Fail-over kernel application for OTA updates. |
| `fs` | `data` | `spiffs` / `littlefs` | `0x820000` | ~7.8 MB (`0x7D0000`) | LittleFS partition for user data storage. |
| `coredump`| `data` | `coredump`| `0xFF0000` | 64 KB (`0x10000`) | Memory logs compiled during a panic event. |

---

## Kernel Subsystems

### VFS (Virtual File System)
The VFS is the central hub for I/O routing. Filesystems and custom nodes implement the [Inode](kernel/src/vfs/inode.rs) trait to interact with standard file handlers.

- **File Descriptors (`Fd`)**: Aliased to `i32`. The global descriptor table (`FdTable` in [vfs/mod.rs](kernel/src/vfs/mod.rs#L27)) permits up to **64** concurrently open descriptors.
- **Mount points**: In Phase 0, VFS mounts a RAM-based volatile filesystem (`RamFs` at [fs/ramfs.rs](kernel/src/fs/ramfs.rs)) at `/` and `/tmp`, and a devices filesystem (`DevFs` at [vfs/devfs.rs](kernel/src/vfs/devfs.rs)) at `/dev`.

#### Supported Device Files in `/dev`
1. `/dev/null`: Discards all inputs; returns EOF on read requests.
2. `/dev/zero`: Provides infinite null bytes (`0`) on read; discards writes.
3. `/dev/console`: Routes read and write operations to the native JTAG UART hardware console.

---

### Task Scheduler
Located in [scheduler/mod.rs](kernel/src/scheduler/mod.rs), the scheduler performs task scheduling for multitasking in the kernel.

- **Task Execution**: Cooperative context switching via `yield_now()` is verified working on hardware; a preemptive path driven by the SYSTIMER interrupt is present and being hardened (see the [roadmap](#project-status-and-development-roadmap)).
- **Quantum**: The running task's time slice is set to **5 ticks**. At a tick rate of `TICK_HZ = 100`, this corresponds to **~50 ms**.
- **Task Lifecycle**: Managed inside [scheduler/task.rs](kernel/src/scheduler/task.rs) through four canonical states:
  - `Ready`: In the scheduler's run queue, waiting to be allocated execution time.
  - `Running`: The thread currently holding the CPU context.
  - `Blocked`: Paused thread waiting for an event (timers, I/O lock).
  - `Zombie`: Terminated task awaiting resource collection by the parent.

---

### Syscall Mechanism (System Calls)
The boundary between userland tasks and kernel operations is managed by a strict register-based ABI ([syscall/mod.rs](kernel/src/syscall/mod.rs)).

Syscalls are dispatched by loading the identifier index into register `a2`, storing inputs in registers `a3` through `a7`, and calling a software trap instruction. If a syscall fails, the kernel returns negative errno values (e.g. `-12` for `ENOMEM`, `-2` for `ENOENT`), translated via the [KError](kernel/src/prelude.rs#L13-L50) enum.

#### Syscall ABI Table (Phase 0 / Phase 6+)

| ID (`a2`) | Syscall Name | POSIX Signature | Description |
| :---: | :--- | :--- | :--- |
| **0** | `Read` | `read(fd, buf_ptr, len) -> bytes` | Reads data from a file descriptor. |
| **1** | `Write` | `write(fd, buf_ptr, len) -> bytes` | Writes data to a file descriptor. |
| **2** | `Open` | `open(path_ptr, path_len, flags) -> fd` | Opens or creates a file at path. |
| **3** | `Close` | `close(fd) -> 0` | Closes and releases file descriptor. |
| **4** | `Ioctl` | `ioctl(fd, cmd, arg)` | Configures device properties. |
| **5** | `Exit` | `exit(code)` | Terminates current thread. |
| **6** | `Spawn` | `spawn(name, entry, arg, stack, prio) -> tid` | Spawns a new task. |
| **7** | `Wait` | `wait(tid)` | Waits for task completion (reserved). |
| **8** | `Seek` | `seek(fd, offset, whence) -> pos` | Offsets open file cursor. |
| **9** | `Mkdir` | `mkdir(path_ptr) -> 0` | Creates a new folder. |
| **10** | `Unlink` | `unlink(path_ptr) -> 0` | Deletes a file or directory. |
| **11** | `Readdir`| `readdir(fd, entry_ptr) -> bytes` | Lists open directory folders. |
| **12** | `UptimeMs`| `uptime_ms() -> ms` | Milliseconds elapsed since boot. |
| **13** | `Sbrk` | `sbrk(incr) -> heap_size` | Modifies memory heap size allocation. |
| **14** | `Yield` | `yield() -> 0` | Voluntarily yields execution time. |

---

### A/B Update Scheme (OTA)
EspressoOS features boot and upgrade structures matching the Espressif dual `otadata` sector format.

The `otadata` partition is split into **2 sectors** of **4 KB** each. Every sector stores an `esp_ota_select_entry_t` entry containing an incremental sequence counter (`ota_seq`) and a CRC-32 checksum. During the boot sequence:
1. The bootloader reads both sectors and verifies signatures and CRC-32 integrity.
2. It identifies the valid sector with the highest sequence number `ota_seq`.
3. It selects the boot slot index using:
   $$\text{Slot Index} = (\text{ota\_seq} - 1) \pmod 2$$
   - `0` selects the `factory` slot (Slot A).
   - `1` selects the `ota_0` slot (Slot B).
4. If no valid sector header is present, it defaults to booting the `factory` partition as a safe fallback.

---

### Peripheral Device Drivers
- **UART JTAG Serial** ([drivers/uart.rs](kernel/src/drivers/uart.rs)): Console reader and writer. Operates asynchronously over native USB JTAG hardware channels.
- **GPIO** ([drivers/gpio.rs](kernel/src/drivers/gpio.rs)): Pin input, output, pull-up, and pull-down configurations.
- **Flash SPI** ([drivers/flash.rs](kernel/src/drivers/flash.rs)): SPI helper routines interacting directly with the chip's internal boot ROM functions to write/erase memory blocks.

### Networking (WiFi + TCP/IP)
The network stack ([drivers/wifi.rs](kernel/src/drivers/wifi.rs)) runs as a single cooperative kernel task (`net_task`) that owns the radio and the TCP/IP engine and yields the CPU on every poll, so it coexists with the shell and heartbeat tasks.

- **Radio**: 802.11 b/g/n **station (STA)** mode via `esp-wifi` 0.12, associating to the SSID/password from `wifi_credentials.rs` (git-ignored; see the [`.example`](kernel/src/wifi_credentials.rs.example) template). esp-wifi runs its own firmware scheduler on `TIMG0` while the kernel scheduler uses `SYSTIMER` — the two coexist without conflict.
- **TCP/IP**: the `smoltcp` 0.12 stack, fed by esp-wifi's `WifiDevice`, with a **DHCPv4** client that prints the assigned IP (`[net] IP = …`).
- **Sockets**: a TCP **echo** server on port **2323** (a simple end-to-end reachability check) and the **SSH** server on port **22**, both driven from the same non-blocking poll loop.
- **Memory**: esp-wifi's buffers are served from the kernel heap; the 8 MB PSRAM is registered as a heap region so the stack has room (internal SRAM is registered first so DMA-capable allocations stay in SRAM).

### SSH Server (Working)
A minimal, from-scratch **SSH-2.0 server** ([drivers/ssh](kernel/src/drivers/ssh)) that serves the interactive REPL shell over TCP port 22. The full handshake is **verified end-to-end against OpenSSH 9.5** on real hardware. All cryptographic primitives are delegated to audited `no_std` crates (RustCrypto / dalek) — the kernel implements the SSH **protocol**, never the crypto itself.

- **Transport (`proto.rs`, `crypt.rs`)**: RFC 4251 base types (uint32, string, name-list, mpint) and the SSH Binary Packet Protocol (RFC 4253 §6), verified by a host-side Python suite ([ssh_proto_tests.py](tools/tests/ssh_proto_tests.py)). Session AEAD uses the **`chacha20-poly1305@openssh.com`** construction (`chacha20` + `poly1305`, standard/unpadded Poly1305 tag, per-direction 512-bit keys, sequence-number nonce).
- **Key Exchange (`kex.rs`)**: `curve25519-sha256` ECDH (`x25519-dalek`) with the exchange hash `H` signed by an **`ssh-ed25519`** host key (`ed25519-dalek`); session keys derived per RFC 4253 §7.2 (`sha2`). Ephemeral keys are seeded from the ESP32-S3 **hardware TRNG** ([crypto_rng.rs](kernel/src/drivers/ssh/crypto_rng.rs)).
- **Authentication (`auth.rs`)**: `password` (constant-time compare via `subtle`) and `publickey` (`ssh-ed25519`, `verify_strict`). Credentials in [config.rs](kernel/src/drivers/ssh/config.rs).
- **Connection (`mod.rs`)**: a non-blocking state machine (VersionExchange → KexInit → Kex → NewKeys → UserAuth → Session) pumped from the network task; `session` channel + `pty-req` + `shell`.
- **Shell Bridge (`shell::remote`)**: a unified `ShellIo` abstraction so the **same** REPL runs on the local console (`ConsoleIo`) and over an SSH channel (`SshChannelIo`).
- **Host key stability**: derived from a fixed dev seed (`HOST_KEY_SEED`) so the fingerprint is constant across reboots (persistent, TRNG-generated keys arrive with LittleFS).

---

## Interactive REPL Shell

EspressoOS starts a CLI shell as a scheduler task on boot. The **same** REPL ([shell/mod.rs](kernel/src/shell/mod.rs), generic over the `ShellIo` trait in [shell/remote.rs](kernel/src/shell/remote.rs)) serves both the local serial console and **remote SSH sessions** — over SSH it is the shell you land in after `ssh <user>@<ip>`.

- **Terminal Control**: Includes support for character deleting (Backspace), execution interruption (`Ctrl-C`), and limits input strings to **256 characters** to prevent stack overflow.
- **Output Redirection**: The parser ([shell/parser.rs](kernel/src/shell/parser.rs)) supports writing redirection `>` (overwrite) and `>>` (append), setting up descriptors on the targeted VFS nodes before calling commands.
- **Pipes**: Pipe operators `|` are parsed by the shell syntax parser, but their inter-process communication logic will be completed during the userland multi-tasking phase.

### Shell Commands Tree

```
EspressoOS CLI Commands
├── System Info
│   ├── help              # Lists all commands and usage guidelines
│   ├── clear             # Clears the screen using ANSI escape codes
│   ├── uptime            # Prints milliseconds elapsed since boot
│   ├── free              # Displays memory usage status of the kernel heap
│   ├── ps                # Lists all active tasks and execution states
│   └── reboot            # Restarts the CPU via software reset
├── Filesystem / VFS
│   ├── pwd               # Prints the current working directory
│   ├── cd [path]         # Changes the working directory (defaults to `/`)
│   ├── ls [path]         # Lists a directory (defaults to the current directory)
│   ├── cat <file>        # Prints file content
│   ├── mkdir <dir>       # Creates a directory
│   ├── touch <file>      # Creates an empty file
│   ├── rm <file>         # Deletes a file or an empty directory
│   └── write <file> <tx> # Writes string data into a file (truncates)
└── Text
    └── echo [-n] <text>  # Prints a line of text (use `-n` to omit newline)
```

The shell maintains a **working directory (CWD)** shown in the prompt (`user@EspressoOS:cwd$ `, with the root `/` displayed as `~`, e.g. `youareme@EspressoOS:/tmp$ `). Filesystem commands accept **relative paths**, which are resolved against the CWD before hitting the VFS (the VFS itself only accepts absolute, normalized paths). Command errors are routed to the active session — including the remote SSH channel — so you see them wherever you're connected.

Example over SSH:
```
youareme@EspressoOS:~$ mkdir demo
youareme@EspressoOS:~$ cd demo
youareme@EspressoOS:/demo$ write hello.txt hola mundo
youareme@EspressoOS:/demo$ cat hello.txt
hola mundo
youareme@EspressoOS:/demo$ ls
hello.txt
youareme@EspressoOS:/demo$ cd ..
youareme@EspressoOS:~$ ls /dev
console@
null@
zero@
```

---

## Verification and Mock Testing

As a bare-metal OS running on target silicon, compiling and running tests directly on the development environment is difficult. To address this, EspressoOS includes **Python validation harnesses** located in [tools/tests](tools/tests).

These harnesses contain offline verification logic for the kernel's pure modules:

### Simulated Algorithms and Checked Tests
1. **Shell Parser & Tokenizer** ([logic_tests.py](tools/tests/logic_tests.py)): Tests parsing of single/double quotes, escape slashes `\`, redirection parsing, and pipe parsing.
2. **VFS Path Normalization** ([logic_tests.py](tools/tests/logic_tests.py)): Validates resolution of relative paths, absolute paths, directory symbols `.`, and folder parent elements `..`.
3. **OTA Selection Logic** ([logic_tests.py](tools/tests/logic_tests.py)): Tests CRC-32 parsing and selection calculations to verify bootloader decisions during power failures.
4. **RamFs File System Semantics** ([logic_tests.py](tools/tests/logic_tests.py)): Simulates reading, writing, and offsets of virtual VFS nodes.
5. **SSH Binary Packet Protocol & Codecs** ([ssh_proto_tests.py](tools/tests/ssh_proto_tests.py)): Verifies RFC 4251 data representations (uint32, string, name-list, mpint) and framing rules (padding, block alignment, lengths) for SSH packets.

### Running Logical Tests

You can run the unified test runner to execute all harnesses:
```bash
python tools/tests/run_all.py
```

Alternatively, run specific suites using:
```bash
# Run shell, VFS, RamFs and OTA tests
python tools/tests/logic_tests.py

# Run SSH protocol codec and framing tests
python tools/tests/ssh_proto_tests.py
```

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
