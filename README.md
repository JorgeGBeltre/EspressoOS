# EspressoOS — A `no_std` Unix-like Operating System in Rust for ESP32-S3

[![Rust Version](https://img.shields.io/badge/Rust-1.75%2B%20%2F%20Xtensa-orange?logo=rust)](https://github.com/esp-rs/rust)
[![Target Platform](https://img.shields.io/badge/Platform-ESP32--S3--WROOM--1-blue?logo=espressif)](https://www.espressif.com/en/products/socs/esp32-s3)
[![License](https://img.shields.io/badge/License-MIT%20or%20Apache--2.0-teal)](LICENSE)
[![Build Status](https://img.shields.io/badge/Phase-0%20(Bring--up)-green)](#project-status-and-development-roadmap)

---

**EspressoOS** is a Unix-like operating system written entirely from scratch in `no_std` Rust for the **ESP32-S3-WROOM-1-N16R8** development board (Xtensa LX7 dual-core, 16 MB flash, and 8 MB PSRAM). 

The system implements fundamental operating system concepts (a Virtual File System "VFS" with an everything-is-a-file abstraction, a preemptive and cooperative multitasking scheduler, kernel-level peripheral drivers, a stable system call "syscall" ABI, and an interactive REPL shell) in a resource-constrained embedded hardware environment.

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
- [Interactive REPL Shell](#interactive-repl-shell)
- [Verification and Mock Testing](#verification-and-mock-testing)
- [Contribution and License](#contribution-and-license)
- [License](#license)
- [Contact](#contact)
- [Support](#support)

---

## Project Status and Development Roadmap

EspressoOS development is structured into **10 incremental phases**. Currently, the project is in **Phase 0 (Bring-up / Skeleton)**.

```
Phase 0 ──────► Phase 1 ──────► Phase 2 ──────► Phase 3 ──────► Phase 4 
Bring-up       Memory         Multitasking   Bus Drivers    Storage & VFS
(Current)      (PSRAM)        (Scheduler)    (I2C/SPI)      (Flash/LittleFS)
  │
  ├──────────► Phase 5 ──────► Phase 6 ──────► Phase 7 ──────► Phase 8 ──────► Phase 9
               OTA A/B        Syscalls/      Networking     Memory         SMP Dual-Core
               Firmware       Userland       (WiFi/smoltcp) Protection     Multiprocessing
```

### Development Phases Roadmap Table

| Phase | Title | Involved Components / Status |
| :--- | :--- | :--- |
| **Phase 0** | **Bring-up (Skeleton)** | **[CURRENT]** Repository directory tree configured. Xtensa CPU clock setup, kernel heap initialization (SRAM), console UART, VFS boot (mounting `/dev` and `/tmp`), scheduler setup with `idle` thread, interactive `shell`, and a `heartbeat` task (blinking LED). |
| **Phase 1** | **Memory Management (PSRAM)** | Enabling mapping and heap integration for the 8 MB external PSRAM using `esp-alloc` as a secondary allocator region ([mm/heap.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/mm/heap.rs)). |
| **Phase 2** | **Task Scheduler** | Fully preemptive multitasking driven by hardware interrupts (SYSTIMER at [timer.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/arch/xtensa/timer.rs)) and Round-Robin scheduler policies at [policy.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/scheduler/policy.rs). |
| **Phase 3** | **Bus Drivers** | Implementing and integrating master bus drivers for I2C ([i2c.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/drivers/i2c.rs)) and SPI ([spi.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/drivers/spi.rs)) at the kernel level. |
| **Phase 4** | **Storage & Filesystems** | Connecting the internal SPI flash NOR driver ([flash.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/drivers/flash.rs)) and mounting **LittleFS** ([fs/littlefs/mod.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/fs/littlefs/mod.rs)) as the primary persistent storage on `/`. |
| **Phase 5** | **OTA A/B Updates** | OTA firmware partition selection, validation (checksum/signatures), writing, and failsafe rollback features inside [ota/partition.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/ota/partition.rs). |
| **Phase 6** | **Syscalls & Userland** | Isolation of system calls and execution of unprivileged user applications with separated kernel and user stacks. |
| **Phase 7** | **Networking (WiFi)** | Enabling the 802.11 radio transceiver using `esp-wifi` bound to the `smoltcp` TCP/IP stack ([drivers/wifi.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/drivers/wifi.rs)). |
| **Phase 8** | **Memory Protection (PMS)** | Configuring the ESP32-S3 PMS (Peripheral Memory System) / World Controller hardware registers to protect kernel address spaces from user space tasks ([mm/mpu.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/mm/mpu.rs)). |
| **Phase 9** | **SMP Dual-Core** | Enabling Symmetric Multiprocessing across both Xtensa LX7 cores (PRO_CPU and APP_CPU) using atomic spinlocks and core affinity policies ([scheduler/core_sync.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/scheduler/core_sync.rs)). |

---

## Key Features

- **Pure Rust `no_std` Development**: Zero dependency on the C-based ESP-IDF framework or standard library; complete low-level control of the hardware.
- **Virtual File System (VFS)**: Uniform abstractions for devices and file operations (`/dev/console`, `/dev/null`, `/dev/zero`, `/tmp`). Standardized file descriptors (`Fd`) supporting `open`/`close`/`read`/`write`/`seek`/`readdir`.
- **Hybrid Task Scheduler**: Supports cooperative multitasking through `yield_now()` and preemptive scheduling triggered by the hardware SYSTIMER interrupt, managing Xtensa register frames directly in assembly ([context.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/arch/xtensa/context.rs)).
- **System Call ABI**: POSIX-like system calls utilizing registers (e.g., `a2` for syscall index) to transition from user mode to kernel mode safely.
- **OTA Updates & Partition Mappings**: Core logic and structures compatible with the Espressif partition table format, prepared for safe, atomic OTA updates.
- **Custom Partition Generator**: Bundled Python utility to generate and inspect bin tables compatible with the hardware ROM bootloader.
- **Logical Verification Simulator**: A comprehensive Python-based mock test harness ([logic_tests.py](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/tools/tests/logic_tests.py)) that mimics the exact behavior of key OS modules for verification in targetless host environments.

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
│   │   │   ├── wifi.rs     # Network driver (esp-wifi + smoltcp) (draft)
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
│       └── logic_tests.py  # Python emulation suite to test OS logic
├── Cargo.toml              # Parent workspace Cargo configuration
├── partitions.csv          # Flash memory layout specification
├── rust-toolchain.toml     # Toolchain pin configuration
└── README.md               # This documentation file
```

---

## Environment Prerequisites

To compile and flash EspressoOS, you will need the Espressif Xtensa Rust toolchain, flashing tools, and Python 3.

1. **Install Rust + Espressif Xtensa Toolchain**:
   Install using the official `espup` tool:
   ```bash
   cargo install espup
   espup install
   ```
   *Note: Follow the terminal instructions to export variables (e.g. `source ~/export-esp.sh` or configure environment variables in Windows).*

2. **Install Flashing Tools (`espflash`)**:
   ```bash
   cargo install espflash
   ```

3. **Hardware Connections**:
   Connect your **ESP32-S3** board to the host PC using the native **USB-Serial-JTAG** port directly (connected to the chip, not via external USB-to-UART bridging ICs if the board has two ports).

---

## Building and Flashing

### 1. Compile the Partition Table
The flash memory must be partitioned before compiling and flashing the operating system. Run the Python generator from the repository root:

```bash
python tools/partition-gen/partition_gen.py
```
This utility parses [partitions.csv](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/partitions.csv) and compiles it into `partitions.bin`, validating partition alignments and boundary limits.

### 2. Build the Kernel
Compile the kernel binary optimized for the Xtensa architecture:
```bash
cargo build --release
```

### 3. Flash and Monitor
Write the binary onto the ESP32-S3 flash and start the serial monitor with a single command:
```bash
cargo run --release
```
*(This command runs `espflash flash --monitor` automatically as configured in `.cargo/config.toml`)*

#### Expected Serial Output (Phase 0 - Bring-up)
```
========================================
   esp32s3-os   ·   kernel
   Consola viva. Arrancando subsistemas.
   Heap del kernel: 65536 bytes
========================================
[kernel] tarea 'shell' creada (tid=1)
[kernel] tarea 'heartbeat' creada (tid=2)

esp32s3-os shell. Escribe 'help' para ver los comandos.
esp32s3-os> 
```
The on-board LED will flash at approximately **1 Hz**. If the LED does not blink, check the `LED_GPIO` constant defined in [main.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/main.rs#L44) and change it to match your board layout (typically GPIO 2 or GPIO 48).

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

The matching flash map layout constants are declared in [prelude.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/prelude.rs#L83-L106):

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
The VFS is the central hub for I/O routing. Filesystems and custom nodes implement the [Inode](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/vfs/inode.rs) trait to interact with standard file handlers.

- **File Descriptors (`Fd`)**: Aliased to `i32`. The global descriptor table (`FdTable` in [vfs/mod.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/vfs/mod.rs#L27)) permits up to **64** concurrently open descriptors.
- **Mount points**: In Phase 0, VFS mounts a RAM-based volatile filesystem (`RamFs` at [fs/ramfs.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/fs/ramfs.rs)) at `/` and `/tmp`, and a devices filesystem (`DevFs` at [vfs/devfs.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/vfs/devfs.rs)) at `/dev`.

#### Supported Device Files in `/dev`
1. `/dev/null`: Discards all inputs; returns EOF on read requests.
2. `/dev/zero`: Provides infinite null bytes (`0`) on read; discards writes.
3. `/dev/console`: Routes read and write operations to the native JTAG UART hardware console.

---

### Task Scheduler
Located in [scheduler/mod.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/scheduler/mod.rs), the scheduler performs task scheduling for multitasking in the kernel.

- **Hybrid Task Execution**: Supports cooperative context switching via `yield_now()` and preemptive switching driven by a periodic timer interrupt from the hardware SYSTIMER.
- **Quantum**: The running task's time slice is set to **5 ticks**. At a tick rate of `TICK_HZ = 100`, this corresponds to **~50 ms**.
- **Task Lifecycle**: Managed inside [scheduler/task.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/scheduler/task.rs) through four canonical states:
  - `Ready`: In the scheduler's run queue, waiting to be allocated execution time.
  - `Running`: The thread currently holding the CPU context.
  - `Blocked`: Paused thread waiting for an event (timers, I/O lock).
  - `Zombie`: Terminated task awaiting resource collection by the parent.

---

### Syscall Mechanism (System Calls)
The boundary between userland tasks and kernel operations is managed by a strict register-based ABI ([syscall/mod.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/syscall/mod.rs)).

Syscalls are dispatched by loading the identifier index into register `a2`, storing inputs in registers `a3` through `a7`, and calling a software trap instruction. If a syscall fails, the kernel returns negative errno values (e.g. `-12` for `ENOMEM`, `-2` for `ENOENT`), translated via the [KError](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/prelude.rs#L13-L50) enum.

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
- **UART JTAG Serial** ([drivers/uart.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/drivers/uart.rs)): Console reader and writer. Operates asynchronously over native USB JTAG hardware channels.
- **GPIO** ([drivers/gpio.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/drivers/gpio.rs)): Digital configuration controller providing Pin input, output, pull-up, and pull-down configurations.
- **Flash SPI** ([drivers/flash.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/drivers/flash.rs)): SPI helper routines interacting directly with the chip's internal boot ROM functions to write/erase memory blocks.

---

## Interactive REPL Shell

EspressoOS starts a CLI shell on the primary console channel upon boot. The REPL loop is located inside [shell/mod.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/shell/mod.rs) and monitors console characters.

- **Terminal Control**: Includes support for character deleting (Backspace), execution interruption (`Ctrl-C`), and limits input strings to **256 characters** to prevent stack overflow.
- **Output Redirection**: The parser ([shell/parser.rs](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/kernel/src/shell/parser.rs)) supports writing redirection `>` (overwrite) and `>>` (append), setting up descriptors on the targeted VFS nodes before calling commands.
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
└── Filesystem / VFS
    ├── ls [path]         # Lists files in a directory (defaults to `/`)
    ├── cat <file>        # Prints file content to console
    ├── mkdir <dir>       # Creates a directory at path
    ├── touch <file>      # Creates an empty file or updates its timestamp
    ├── rm <file>         # Deletes a file or an empty directory
    ├── write <file> <tx> # Writes string data into a file
    └── echo [-n] <text>  # Prints a line of text (use `-n` to omit newline)
```

---

## Verification and Mock Testing

As a bare-metal OS running on target silicon, compiling and running tests directly on the development environment is difficult. To address this, EspressoOS includes a **Python validation harness** ([logic_tests.py](file:///C:/Users/Jorge/Desktop/Firmware/EspressoOS/tools/tests/logic_tests.py)).

This harness contains Python translations of the kernel's critical algorithms, allowing developers to verify OS logic offline:

### Simulated Algorithms and Checked Tests
1. **Shell Parser & Tokenizer**: Tests parsing of single/double quotes, escape slashes `\`, redirection parsing, and pipe parsing.
2. **VFS Path Normalization**: Validates resolution of relative paths, absolute paths, directory symbols `.`, and folder parent elements `..`.
3. **OTA Selection Logic**: Tests CRC-32 parsing and selection calculations to verify bootloader decisions during power failures.
4. **RamFs File System Semantics**: Simulates reading, writing, and offsets of virtual VFS nodes.

### Running Logical Tests

Execute the validation suite in your local development workspace using:
```bash
# Via native Python unittest
python tools/tests/logic_tests.py

# Or via pytest
pytest tools/tests/logic_tests.py
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
