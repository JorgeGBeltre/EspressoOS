# EspressoOS — A `no_std` Unix-like Operating System in Rust for ESP32-S3

[![Rust Version](https://img.shields.io/badge/Rust-Xtensa%20(esp)-orange?logo=rust)](https://github.com/esp-rs/rust)
[![Target Platform](https://img.shields.io/badge/Platform-ESP32--S3--WROOM--1--N16R8-blue?logo=espressif)](https://www.espressif.com/en/products/socs/esp32-s3)
[![License](https://img.shields.io/badge/License-MIT)](LICENSE)
[![Status](https://img.shields.io/badge/Status-Interactive%20shell%20%2B%20WiFi%20%2B%20SSH%20on%20hardware-brightgreen)](#2-status--running-on-hardware)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/JorgeGBeltre/EspressoOS)

---

**EspressoOS** is a Unix-like operating system written entirely from scratch in `no_std` Rust for the **ESP32-S3-WROOM-1-N16R8** development board (Xtensa LX7 dual-core, 16 MB flash, 8 MB PSRAM).

It behaves *"like Linux, but for the ESP32-S3"*: preemptive multitasking with a hand-written Xtensa context switch, a Virtual File System (everything-is-a-file) with `/dev`, `/proc` and `/sys`, kernel device drivers reached through a single `ioctl` pattern, a frozen 30-call syscall ABI, **ELF userland programs that execute from PSRAM** (relocated at load time because the LLVM Xtensa backend refuses PIC), a Wi-Fi + TCP/IP stack (`esp-wifi` + `smoltcp`), an **SSH-2.0 server**, a BLE advertiser, and interactive shells reachable both over the **serial console** and over **SSH** — with runtime **Wi-Fi management from the shell** and credentials that persist in flash NVS.

All runtime output and both shells are in **English**; the whole system identifies itself as **EspressoOS** (SSH ident `SSH-2.0-EspressoOS_0.1`, BLE advertising name `EspressoOS`).

The project is mid-way through the **SP2→SP4 "total parity" mandate** (§7): slices R0–R6 have landed and are hardware-verified; the current front is process-control (slice #14), then R7–R11. This README documents the system **as it actually is today**, including the parts that are stubs, latent bugs, or decided-but-not-yet-written.

---

## Table of Contents

- [1. Hardware Target](#1-hardware-target)
- [2. Status — Running on Hardware](#2-status--running-on-hardware)
- [3. Architecture by Subsystem](#3-architecture-by-subsystem)
  - [3.1 Boot sequence](#31-boot-sequence)
  - [3.2 Memory (heap · PSRAM · MMU · stacks · watermark)](#32-memory-heap--psram--mmu--stacks--watermark)
  - [3.3 Scheduler & processes (Model B switch · preemption · SMP · signals)](#33-scheduler--processes-model-b-switch--preemption--smp--signals)
  - [3.4 The syscall ABI (full table)](#34-the-syscall-abi-full-table)
  - [3.5 VFS · /dev · /proc · /sys · pipes · sockets · filesystems](#35-vfs--dev--proc--sys--pipes--sockets--filesystems)
  - [3.6 Drivers (Wi-Fi/net_task · SSH · BLE · the D-1 ioctl bus pattern)](#36-drivers-wi-finet_task--ssh--ble--the-d-1-ioctl-bus-pattern)
  - [3.7 Userland (ELF loader · libc · the shell · every /bin)](#37-userland-elf-loader--libc--the-shell--every-bin)
- [4. Build & Flash](#4-build--flash)
- [5. Command Reference](#5-command-reference)
- [6. The Shells — prompts, redirection, pipes, sessions](#6-the-shells--prompts-redirection-pipes-sessions)
- [7. The SP2→SP4 Mandate — status & decisions](#7-the-sp2sp4-mandate--status--decisions)
- [8. Known Issues & Technical Debt](#8-known-issues--technical-debt)
- [9. Operational Notes](#9-operational-notes)
- [Repository Structure](#repository-structure)
- [Memory Map & Partition Table](#memory-map--partition-table)
- [License](#license)
- [Contact](#contact)
- [Support](#support)
---

## 1. Hardware Target

| Item | Value |
| :--- | :--- |
| Board | ESP32-S3-WROOM-1-**N16R8** dev board |
| CPU | Xtensa **LX7**, dual-core (ProCpu + AppCpu), `CpuClock::max()` |
| Flash | **16 MB** external SPI NOR |
| PSRAM | **8 MB** octal (`octal-psram`) — requires a **release** build |
| Console | **UART0** via the on-board USB-to-UART bridge (this board wires a **CH343**, USB `1a86:55d3`, appearing as e.g. `COM5`) |
| Radio | 2.4 GHz Wi-Fi (STA) + Bluetooth LE, via `esp-wifi` |
| Toolchain | Espressif `esp` Rust fork (Xtensa target `xtensa-esp32s3-none-elf`), `esp-hal 0.23.1` |

> The console is on UART0 (`esp-println` `uart` feature), **not** the native USB-Serial-JTAG, because this board routes a CH343 to UART0. If your board exposes only USB-Serial-JTAG, switch the `esp-println` feature in `kernel/Cargo.toml` from `uart` to `jtag-serial`.

---

## 2. Status — Running on Hardware

EspressoOS **boots and runs on a physical ESP32-S3**, obtains an IP over Wi-Fi/DHCP, is reachable over SSH, and drives interactive shells over both the serial console and the network. The default build activates **only the `syscall-trap` feature** — it is deliberately the known-good image (single-core, no hardware memory protection; see the feature gates in §4).

### Hardware-verified

| Capability | Notes |
| :--- | :--- |
| Compiles & links for `xtensa-esp32s3-none-elf` (`--release`) | Release is mandatory (PSRAM + esp-wifi don't work in debug) |
| Boot: HAL init, 128 KB internal heap, VFS mounts (`/` espfs, `/tmp` ramfs, `/dev` devfs, `/proc` procfs, `/sys` sysfs) | ✅ |
| **8 MB PSRAM** — ~7 MB into the kernel heap, 1 MB reserved and **executable on the instruction bus** (selftest returns 42) | ✅ |
| **Preemptive multitasking** — 100 Hz SYSTIMER, 50 ms quantum, "Model B" in-frame context switch | ✅ |
| **Userland ELF exec from PSRAM** with argv, 32-slot pool, load-time relocation (no PIC) | ✅ two instances of the same binary coexist |
| **Wi-Fi STA + DHCP + TCP/IP** (`esp-wifi` + `smoltcp`), runtime scan/connect/disconnect | ✅ `wifi connect` works, persists to NVS |
| **SSH-2.0 server** (curve25519-sha256 · ssh-ed25519 · chacha20-poly1305@openssh) | ✅ `ssh youareme@<ip>` (password auth) |
| **Persistent `EspFs` on `/`** (log-structured, survives power cycle & reflash) | ✅ |
| **Per-session I/O** — serial console and each SSH session own a pid, an fd table, and a `SessionChannel` | ✅ |
| **Hardware SHA-256** via `/dev/sha0` (`sha256 hello` == the public digest) | ✅ strong differential |
| **`reboot`** via `/dev/power` (`rst:0x3 RTC_SW_SYS_RST`, clean restart) | ✅ |
| **BLE advertise** as `EspressoOS` via `/bin/ble advertise` (D-4 async path, coexists with Wi-Fi+SSH >1h) | ✅ from **serial**; radio-discoverable row pending a scanner |
| `/proc` (uptime/meminfo+slots/stacks/tasks/net/sockets/`<pid>`), `/sys` (kernel/smp/pms) | ✅ |
| Your `partitions.csv` is the table on the chip (6 entries, kernel at `0x20000`) | ✅ |

### Mechanism-only / partial / in-progress

| Capability | Notes |
| :--- | :--- |
| **I2C `/dev/i2c0`, SPI `/dev/spi0`** | Driver + ioctl frontier verified, but data path only against an **empty bus** (both return zeros). Needs a live device (e.g. SSD1306 @ `0x3c`) to fully close. |
| **`power sleep`** (light sleep) | Does **not** reliably resume — diagnosed as a **pre-existing platform limitation** (the kernel builtin hangs identically). `deep-sleep`/`reboot` are the reliable paths. |
| **SSH → shell** | Today SSH runs the **kernel builtin shell** (the "oracle"); serial runs the userland `/bin/sh`. R7.4 will point SSH at `/bin/sh` and R10 retires the kernel shell. |
| **OTA A/B update** | Built into the default image (TCP :3300 receiver on every boot), but **never verified end to end**. |
| **PMS memory protection** (`--features pms`) / **SMP dual-core** (`--features smp`) | Real implementations exist but are **off by default** — the default image has no hardware stack-guard, no World-0/World-1 isolation, and is single-core (ProCpu only). |
| **littlefs** | Empty stub (validates a region, presents an empty read-only root); not mounted anywhere. |

---

## 3. Architecture by Subsystem

Module tree (declared in `kernel/src/main.rs`): `arch, drivers, fs, mm, ota, prelude, scheduler, session, shell, syscall, vfs, wifi_credentials`. The userland ELF blobs are embedded at build time via `include!(concat!(env!("OUT_DIR"), "/userland_bin.rs"))`.

### 3.1 Boot sequence

`kernel/src/main.rs` — `#[esp_hal::main] fn main() -> !`. Ordered steps:

1. **`esp_hal::init`** with `CpuClock::max()` and `PsramConfig { size: 8 MiB }`.
2. **PSRAM carve-up** (`psram_raw_parts`): reserve **1 MiB** at the PSRAM base for the userland executable slot pool (`psram_exec::set_data_base`); hand the remaining ~7 MiB to the heap (`mm::heap::add_psram` then `mm::heap::init`).
3. **PSRAM-exec bring-up**: `map_instruction(0, 16 pages)` (1 MiB / 64 KiB), then `selftest()` copies a 2-instruction `movi a2,42 / ret` template into a slot, syncs caches, `callx0`s the instruction-bus alias, and expects `42`. Prints `OK: code EXECUTED from PSRAM returned 42`.
4. `drivers::power::init(LPWR)`, `drivers::device::init()` *(the vestigial/dead registry — see §3.5)*, `drivers::uart::init()`.
5. `banner()` (prints kernel heap size), `arch::xtensa::interrupts::init()` (reads VECBASE), `mm::mpu::init()` (no-op unless `pms`).
6. **VFS mounts**: `vfs::init()`; mount `/dev` (devfs); flash capacity check vs `layout::FLASH_SIZE` (warns if the image header says < 16 MB — see §4); mount `/` from `EspFs::mount()` (**falls back to ramfs** on failure); mount `/tmp` (ramfs), `/proc` (ProcFs), `/sys` (SysFs).
7. **Seed pid-0 fd table**: `resolve("/dev/console")` **outside** the fd lock, then `seed_fd_table(0, console)`. Every task with no process of its own shares pid 0's table (0/1/2 → serial console).
8. **`install_userland()`** — deploy `/bin/*` by content-diff (§3.7, §4).
9. **`init_etc_files()`** — rewrite `/etc/rc` every boot; warn loudly if `/etc/passwd` exists (it overrides the compiled SSH credential and persists across reflash).
10. `drivers::i2c::init(I2C0, GPIO8, GPIO9)`, `drivers::spi::init(SPI2, GPIO12, GPIO11, GPIO13)`.
11. **`scheduler::init()`** (builds the scheduler + the idle task), create serial session 0 (`session::create(Uart, None)`).
12. **Spawn tasks** (spawn-blocked → register process → seed fd table → unblock, so a task never runs before it owns a pid/fd table):

    | Task | Entry | Stack | Notes |
    | :--- | :--- | :--- | :--- |
    | `idle` (tid 0) | `idle_entry` | 16 KB | affinity core 0; `spin_loop` + yield |
    | `init-sup` | `init_supervisor_task` | 16 KB | pid-owning; runs `/bin/init`→`/bin/sh`, falls back to the **kernel shell** if init dies |
    | `heartbeat` | `heartbeat_task` | 16 KB | blinks the LED on **GPIO2** every 500 ms |
    | `net` | `drivers::wifi::net_task` | **24 KB** | affinity forced to core 0; the deepest worker |

13. **Wi-Fi**: `provide_peripherals(TIMG0, RNG, RADIO_CLK, WIFI, BT)`, then the `net` task runs the two-phase Wi-Fi bring-up.
14. `arch::xtensa::timer::init()` starts the **100 Hz** SYSTIMER that drives preemption. Optional `#[cfg(smp)]` core-1 bring-up.
15. **`scheduler::run()`** — never returns; launches the first ready task via `resume_task`.

`init_supervisor_task` is the PID-1-style supervisor: it runs `/bin/init` as a child (inheriting fds 0/1/2), and if init exits or never starts it falls to `shell::run_session(None)` (the kernel builtin shell) so the board is never without a console. The **serial console therefore primarily runs the userland `/bin/sh`**; the kernel shell is only the fallback (SP1 dual-boot). This fallback is marked for removal in R10.

### 3.2 Memory (heap · PSRAM · MMU · stacks · watermark)

**The single source of truth for the memory/flash map is `kernel/src/prelude.rs::layout`.**

- **Kernel heap** (`mm/heap.rs`): `esp_alloc::HEAP` with two regions — a static **128 KiB internal SRAM** buffer (`MemoryCapability::Internal`) and **~7 MiB PSRAM** (`MemoryCapability::External`). Both go into the same global allocator; **task/process stacks are heap allocations**, so a stack may land in PSRAM or SRAM. `stats()` (`total/used/free`) feeds `free` and `/proc/meminfo`.
- **PSRAM executable slot pool** (`mm/psram_exec.rs`): the reserved 1 MiB (`USER_REGION_SIZE = 0x10_0000`) is split into a low **512 KB text** image (executed through the instruction-bus alias `USER_IBUS_BASE = 0x4280_0000`) and a high **512 KB data** image (addressed directly at the runtime-probed data base; userland links `.data` at `LINK_DATA = 0x3c17_0000`). **32 slots × 16 KB** (`SLOT_COUNT = 32`, `SLOT_SIZE = 16 KiB`), tracked by a single `AtomicU32` bitmap; `slot_alloc()` is a **lock-free CAS loop** (deliberately not a Mutex — flipping a bit shouldn't disable interrupts). Split-Harvard address helpers: `slot_text_exec` (execute, read-only), `slot_text_write` (the **data alias** you must write code through), `slot_data`. `sync_caches()` = `Cache_WriteBack_All` then `Cache_Invalidate_ICache_All` (order matters — code is written through the data alias, executed through the instruction alias). Compile-time asserts tie the geometry to the linker script emitted by `kernel/build.rs`. **Only 32 slots exist — a process whose slot isn't returned on reap leaks it for the boot.**
- **MMU/PMS/stack-guard** (`mm/mpu.rs`): the real implementation (`#[cfg(feature = "pms")]`) drives the ESP32-S3 SENSITIVE PMS monitor, World-0/World-1 constrain fields, W^X, and per-core `ASSIST_DEBUG` SP min/max stack guards, and arms the WCL world-controller on each switch. **In a default build `pms` is off**, so `configure_stack_guard`/`prepare_world_switch` compile to nothing: **no hardware stack-overflow trap and no privilege separation.** User/kernel separation is only the `is_user` bit plus `validate_user` pointer checks.
- **Stacks & the watermark rule** (`scheduler/task.rs`): every stack is painted with `0xDEAD_BEEF` on creation; `stack_high_water()` scans from the base counting intact paint words. `DEFAULT_STACK_SIZE = 16 KiB` (raised from 8 KB — see below); `NET_STACK_SIZE = 24 KiB`; SSH session shells use **8 KiB**. The **25%-free margin invariant**: no slice closes if any task's watermark shows `free < 25%` of its size; this is why `net` was raised 16 K→24 K. Reported live via `/proc/stacks` and `/proc/tasks`.

> **Why 16 K, not 8 K.** On Xtensa the syscall/exception **runs on the interrupted task's own stack**, so a user task's stack must hold its own frames *plus* the kernel's during a syscall. `spawn` (load_elf + relocation + write_argv + register_process) is the deepest path; 8 K overflowed there and corrupted register-spill slots — which *looked like* a context-switch bug but was not (the scheduler preserves registers, verified on hardware). `/bin/sh` measured 9160/16384 during spawn.

### 3.3 Scheduler & processes (Model B switch · preemption · SMP · signals)

**Scheduler** (`scheduler/mod.rs`, `task.rs`, `policy.rs`): `Scheduler { tasks: BTreeMap<Tid, Box<Task>>, ready: Vec<Tid>, current, idle, next_tid, slice_remaining }` behind `static SCHED: Mutex<Option<Scheduler>>`. `TaskState { Ready, Running, Blocked, Zombie }`.

- **Policy is FIFO + affinity, not priority.** `policy::next_ready` scans `ready` for the first task whose affinity matches the core. `Task.priority` is stored (idle=0, default=1, exec=10) but **nothing consumes it** — it is round-robin.
- **Preemption**: SYSTIMER at `TICK_HZ = 100` (10 ms) → `scheduler::tick()` decrements `slice_remaining` (`QUANTUM_TICKS = 5` → **50 ms quantum**) and sets `need_resched`. The level-1 interrupt dispatcher (`arch/xtensa/interrupts.rs::__level_1_interrupt`) runs esp-hal's `handle_interrupts`, then if `need_resched` calls `preempt_switch`, then `check_signals`.
- **Model B context switch** (`preempt_switch`): rather than a software register save, the switch **mutates the live saved exception/interrupt frame in place** — `cur.context.frame = *save_frame` (save), pick next, then `*save_frame = next.context.frame` (restore). The actual register/PC/PS reload happens when the ISR epilogue returns. `resume_task` (raw asm, resets WINDOWBASE/WINDOWSTART, `rfe`) is used **only** for the first task on each core. The scheduler rides the exception/interrupt frame; it never does a cooperative register save.
- **The Mutex is load-bearing and non-reentrant** (`arch/xtensa/sync.rs`): `lock()` disables interrupts (RSIL 15) for the guard's entire lifetime; taking the same Mutex twice on one core **wedges silently** (interrupts off, no panic). Much of `process.rs`/`scheduler.rs` computes values *outside* the guard specifically to avoid nesting SCHED under PROCESS_TABLE (e.g. `current_stack_range`, `cwd_set`, `register_process`).
- **SMP** (`scheduler/core_sync.rs`, all `#[cfg(feature = "smp")]`, **off by default**): starts the APP_CPU with an 8 KB static stack, `run_secondary()` first launch, `current1`/`idle1`, per-core `NEED_RESCHED`/`RESTART_SYSCALL`. Default builds are strictly single-core.

**Processes** (`scheduler/process.rs`): `Process { pid, parent_pid, main_task: Tid, name, state, exit_code, children, slot: Option<SlotIndex>, cwd, pending_signals, signal_handlers[32], signal_restorers[32], saved_signal_context }` in `static PROCESS_TABLE: Mutex<ProcessTable>` with a private `by_tid` reverse index (O(log n) `pid_of_tid`). `register_process` seeds cwd from the parent and `clone_fd_table`s stdio; `reap` frees the PSRAM slot and fd table; `reap_orphans` sweeps parentless zombies (SSH session shells no one `wait()`s for). There is **no `fork`/`exec`** — a spawned child is set up while blocked and then unblocked.

**Signals** (`check_signals`, `sys_kill`, `sys_sigaction`, `sys_sigreturn`): fully implemented kernel-side. Default action for **SIGINT(2)/SIGKILL(9)/SIGTERM(15)** with no handler is `exit(-sig)`; SIGKILL is uncatchable. A handler is entered by rewriting the frame (`PC=handler, A2=sig, A0=restorer`) and popped by `sys_sigreturn`. **But nothing exposes it to the user**: there is no `/bin/kill`, no builtin, and it is not in the build's `APPS` list — so a spinning process is a reset today, exactly as if signals didn't exist. `check_signals` is called from both the interrupt and syscall-return paths **with no user/kernel guard** — inert today (`pending_signals` is always 0), but a latent mine the moment `kill` is exposed. This is the subject of **slice #14** (§7).

### 3.4 The syscall ABI (full table)

Stable ABI: number in `a2`, args in `a3..a8`, return in `a2`. With `--features syscall-trap` (default) `syscall::invoke` emits a real `syscall` instruction and the Xtensa exception path (`syscall/trap.rs`) dispatches; without it, `invoke` calls `dispatch` directly. The table is **frozen at 0..=29** (`syscall/table.rs`) — D-5 of the mandate: no new syscalls in SP2 (R0–R5 added zero; new drivers extend via `ioctl`).

| # | Call | # | Call | # | Call |
|---|------|---|------|---|------|
| 0 | `Read` | 10 | `Unlink` | 20 | `Listen` |
| 1 | `Write` | 11 | `Readdir` | 21 | `Accept` |
| 2 | `Open` | 12 | `UptimeMs` | 22 | `Connect` |
| 3 | `Close` | 13 | `Sbrk` | 23 | `GetTimeOfDay` |
| 4 | `Ioctl` | 14 | `Yield` | 24 | `SetTimeOfDay` |
| 5 | `Exit` | 15 | `Signal` (sigaction) | 25 | `OtaState` |
| 6 | `Spawn` | 16 | `Kill` | 26 | `Pipe` |
| 7 | `Wait` | 17 | `Sigreturn` | 27 | `Dup2` |
| 8 | `Seek` | 18 | `Socket` | 28 | `Chdir` |
| 9 | `Mkdir` | 19 | `Bind` | 29 | `Getcwd` |

Notes worth knowing (all from `syscall/handler.rs`):

- **No `dup`, `fork`, `execve`, `stat`, or `mmap`.** `vfs::dup` exists as a kernel function but has no syscall number.
- **`Ioctl` (4) is the extensibility escape hatch** — every new device driver adds zero syscalls (the recurring "D-1" pattern, §3.6).
- **`Sbrk` (13) does not grow a heap** — it returns `mm::stats().free` (free bytes). Misnamed vs POSIX; unused by any `/bin`.
- **`Spawn` (6)** loads an ELF into a PSRAM slot, writes argv into the child's slot, `spawn_blocked` (prio 10, `is_user=true`), registers the process, then unblocks. The old "raw entry point" form was removed as an arbitrary-kernel-execution hole. Empty argv defaults to `[path]`.
- **`Wait` (7)** reaps a zombie child (writes exit code, frees the slot, cleans up fds); if none is ready it `block_current_noswitch()` + sets the restart flag so the syscall re-runs after wakeup.
- **`Getcwd` (29)** has no ERANGE — a too-small buffer returns `InvalidArgument` (a documented deliberate deviation). **`Chdir` (28)** validates existence + is-a-directory before setting the cwd.
- **Pointer validation (`validate_user`)** is by **mode, not process**: a kernel task passes any non-null address; a user task's pointer must fall inside exactly two regions — its own **stack** (`current_stack_range`) or its own **data slot** (`slot_data`, excluding the text slot). Anything else → `Fault` (EFAULT). Made `pub(crate)` so drivers reuse it for D-1 ioctl structs.
- **`KError` → errno** (`prelude.rs`): all 18 variants map to Linux-style negatives — `NotFound -2`, `WouldBlock -11`, `IoError -5`, `BadFd -9`, `NoMem -12`, `PermissionDenied -13`, `Fault -14`, `Busy -16`, `AlreadyExists -17`, `NotADirectory -20`, `IsADirectory -21`, `InvalidArgument -22`, `TableFull -23`, `NoSpace -28`, `NameTooLong -36`, `Corrupt -84`, `NotSupported -95`, `Timeout -110`.

### 3.5 VFS · /dev · /proc · /sys · pipes · sockets · filesystems

**Core** (`vfs/`): `trait Inode: Send + Sync` (required `kind/size/read_at/write_at`; provided `truncate/ioctl/bind/listen/accept/readdir/lookup/create/unlink/sync/as_socket/...`). `OpenFlags`: `RDONLY 0x1, WRONLY 0x2, RDWR 0x3, CREATE 0x100, APPEND 0x200, TRUNC 0x400`. `MAX_OPEN_FILES = 64` per pid.

- **Per-process fd tables** (`static PROCESS_FD_TABLES: Mutex<BTreeMap<Pid, FdTable>>`). A missing table is an **error** (BadFd), never conjured. Pidless kernel tasks resolve to pid 0. **`read`/`write` do no I/O under the lock**: they snapshot `(inode, offset, perms)` under the guard, **drop it**, do `read_at`/`write_at` unlocked, then `commit_offset` (guarded by `Arc::ptr_eq`). The lock disables interrupts, so a blocking inode under it would wedge the kernel.
- **Path resolution** (`vfs/mount.rs`): `MOUNTS: Mutex<Vec<MountPoint>>`; `resolve(path)` normalizes, picks the **longest-prefix mount**, then walks `lookup` per component. `normalize` applies the caller's cwd (a task with no process → `InvalidArgument`, no silent `/`). Mirrored by `tools/tests/logic_tests.py` (the kernel has no lib target, so `#[cfg(test)]` can't run in-kernel — the Python port is the real test).

**`/dev` (devfs)** registers, in readdir/ino order: `null`, `zero`, `console` (→ UART), `i2c0`, `spi0`, `wlan0`, `sha0`, `power`, `ble0`. The live `Device` trait lives in `vfs/devfs.rs` (`read/write/ioctl`, `off: u64`, ioctl → `usize`). *There is a second, dead `Device` trait in `drivers/device.rs` (`offset: usize`, ioctl → `i32`) with its own registry — `init()` runs at boot but nothing queries it; it duplicates null/zero/console. Vestigial scaffolding, flagged for cleanup.*

**`/proc` (procfs, synthesized read-only)**: `/proc/uptime`, `/proc/meminfo` (`MemTotal/Used/Free` + `SlotsTotal/SlotsUsed` for the PSRAM exec pool), `/proc/stacks` and `/proc/tasks` (both = `stacks_report()`: tid/name/state/used/size/free per task), `/proc/net/sockets` (iterates the live smoltcp `SocketSet`: `Fd Type Local Remote State`), and `/proc/<pid>/status` per live pid.

**`/sys` (sysfs, synthesized read-only)**: `/sys/kernel` (`EspressoOS Kernel v0.1.0`), `/sys/smp` (core id; `smp: disabled` unless `--features smp`), `/sys/pms` (`mm::mpu::report()` under `--features pms`, else `disabled`). State-by-read (D-8); the feature-gated *actions* stay in the kernel shell.

**Pipes** (`vfs/pipe.rs`): the *blocking* counterpart to the console. `read_at`/`write_at` drain/fill, wake the opposite party, and on empty/full **enqueue the Tid then `block_current_noswitch()` while still holding the buffer lock**, then drop and `yield_now()` — so a wakeup can't be lost in the gap. EOF when the last writer drops; EPIPE-like `IoError` when the last reader drops.

**Sockets** (`vfs/socket.rs`, `SocketInode`) — the **userland** socket path (the `socket()` syscall). Supports `AF_INET` + TCP(stream)/UDP(dgram). TCP `connect` pushes `NetCmd::Connect` onto the net task's queue; `bind/listen/accept`. **Today `accept`, `read_at`, `write_at`, and the `sys_connect` TCP wait all spin-with-yield** (lock `NET_SOCKETS`, `yield_now()` in a loop). Slice #14 **decision (D)** changes these to return `WouldBlock` (matching the console/pipe convention) — **not yet implemented** (verified: they still spin). This is architecturally separate from the ECHO/SSH/OTA listeners, which the net task owns directly by handle.

**Filesystems** (`fs/`):

- **espfs** (`fs/espfs/`, the persistent root) — a **log-structured filesystem on raw NOR flash** at `FS_OFFSET = 0x82_0000` (~8 MB). Two superblock sectors (atomic commit) + two log halves (compaction ping-pong). 16-byte record headers (`MAGIC 0xE5F5`, seq, len, CRC-32) + payload; record types `MkFile/MkDir/Write/Truncate/Unlink`. Files are **extents pointing at flash offsets** — data is not copied into RAM; reads zero-fill holes and `flash::read` each extent. `compact()` erases the other half, replays the live tree in `COMPACT_CHUNK=2048` writes, then commits the superblock **before** mutating in-memory state (so a failed commit leaves both flash and memory describing the old half). Mount = load-or-format; replay stops at the first bad/blank record (the log tail). *Known latent debt: a compacted image can be larger than the source (holes materialize as explicit zeros, each chunk gets its own header), so `compact` can hit `NoSpace` and, because every future `append` re-triggers the same overflowing compaction, the fs can **wedge for good**. Doesn't bite at current sizes (~4 MB half vs ~180 KB live) — the "espfs compaction hang".*
- **ramfs** (`fs/ramfs.rs`) — backs `/tmp`, and the `/` fallback if espfs mount fails. Full read/write/truncate/readdir/lookup/create/unlink; `try_reserve` → NoMem on OOM.
- **procfs / sysfs** — as above.
- **littlefs** (`fs/littlefs/`) — **stub**: validates a flash region and presents an empty read-only root (`readdir → None`, `lookup → NotFound`). No real driver; not mounted. Dead scaffolding.

> **The `readdir`-doesn't-list-mounts bug (H4).** `vfs::readdir` resolves the path to the single underlying filesystem and iterates only *that inode's* entries — `MOUNTS` is not consulted to inject nested mount points. So **`ls /` does not show `/dev`, `/tmp`, `/proc`, `/sys`** (they are separate mounts, not directories in the espfs root). They are reachable only by naming them (`ls /proc` works because `resolve("/proc")` matches the mount directly).

### 3.6 Drivers (Wi-Fi/net_task · SSH · BLE · the D-1 ioctl bus pattern)

**Wi-Fi + `net_task`** (`drivers/wifi.rs`) — the single owner of the Wi-Fi controller and the TCP/IP service loop. `provide_peripherals` stashes the peripherals; the `net` task (24 KB stack) takes them and runs a **deliberate two-phase boot**:

- **Phase 1 — association wait, NO smoltcp mounted.** Drains `WIFI_CMD_QUEUE` (so `wifi connect/scan/disconnect` work before an IP exists), retries `connect()` every 3 s, prints "no Wi-Fi yet; system is up" at ~5 s. Rationale: mounting/polling smoltcp over the esp-wifi device **before association hangs the driver**. This lets the board boot even with bad/absent boot creds.
- **Phase 2 — interface up.** Builds a smoltcp `Interface` + `SocketSet` + DHCPv4 socket, published to `static NET_SOCKETS`. **ECHO (2323), SSH (22), OTA (3300)** are `tcp::Socket`s added directly into the set and `listen`ed. The main loop, per iteration: drain wifi cmds → `ble::poll_advertise()` → drain userland `NetCmd`s → `iface.poll` **only when associated** → DHCP events (publish `CURRENT_IP`, print `[net] IP = …`) → ECHO → `reap_orphans()` → drive `ssh_conn.pump(TcpTransport{sock})` → OTA receive-and-buffer → link check → `yield_now()`.
- **`/dev/wlan0`** (D-1): `read()` returns a stable text snapshot (`state:`/`ssid:`/`ip:`/`scan:`/`ap:` rows) consumed by `/bin/wifi`, `/bin/ip`, `/bin/nmcli`; `ioctl` commands `WLAN_NOP=0`, `WLAN_CONNECT=1` (`WlanConnectReq{ssid_ptr,ssid_len,pass_ptr,pass_len}`, SSID ≤32, pass ≤64), `WLAN_DISCONNECT=2`, `WLAN_SCAN=3`. `connect` auto-persists to NVS.
- **Credential store** (`drivers/wifi_store.rs`): a single 128-byte `EWC1` record in the NVS sector at **`0x9000`** (untouched by esp-wifi; `phy_init` at `0xF000` is deliberately left alone). Boot **prefers the saved record** over the compiled `wifi_credentials.rs`. `save`/`load`/`clear`.

**SSH-2.0 server** (`drivers/ssh/`) — single-session, server-only, fixed algorithm set: kex **curve25519-sha256**, host key **ssh-ed25519**, cipher **chacha20-poly1305@openssh.com** (AEAD, MAC none), compression none. `Connection` state machine (`VersionExchange → KexInit → Kex → Encrypted → UserAuth → Session → Closed`) driven one iteration per `pump()`. Password auth reads `/etc/passwd` first (plaintext, constant-time compare) then falls back to the compiled `DEV_USER`/`DEV_PASSWORD` (`ssh/config.rs`). Publickey auth is wired but **cannot succeed** — `authorized_key_blobs()` returns an empty Vec. Host key is derived from a fixed `HOST_KEY_SEED` (stable fingerprint across reboots/reflash; dev-grade). All entropy from the ESP32-S3 **TRNG** (`HwRng`) — a hard rule is **no `getrandom` anywhere** (no bare-metal xtensa backend). A `shell` request builds a `SessionShell` (8 KB stack) → `session::create(Ssh)` → `spawn_blocked` → `register_process` → `seed_fd_table` → `unblock`; `ssh_shell_entry` calls `shell::run_session(user)` — **the kernel builtin shell** (the oracle, to be replaced by `/bin/sh` in R7.4). Dev tooling notes: `crypto_smoke::smoke()` and `announce_host_key()` are defined but **never called**.

**BLE** (`drivers/ble.rs`) — advertises as `EspressoOS`. **D-4 async model**: the `BLE_ADVERTISE=0` ioctl only **enqueues** an atomic flag; `net_task` runs the blocking HCI writes in `poll_advertise()` where the esp-wifi runtime is live. The old synchronous `start_advertising` (HCI I/O on the caller's task) hung the board — that mine is fixed only in the userland `/bin/ble`; **the kernel-shell `ble advertise` builtin still calls the synchronous path** (so over SSH: `ble status` only). A `BLE_ADVERTISE_SYNC_DIAG = 0xD1A6` ioctl exists **only under `--features diag-ble-sync`** (a stack-overflow experiment; default build treats it as `InvalidArgument`).

**The D-1 / D-2 / D-3 bus/device pattern** — every data-carrying driver command is `open("/dev/<node>")` → `ioctl(cmd, arg)` where `arg` is a **pointer to a `#[repr(C)]` request struct**. The kernel validates the struct with `validate_user(arg, size_of::<Req>())`, range-checks lengths (D-2 caps), validates **each embedded pointer**, then bounces data through a fixed kernel buffer. **D-3**: state via `read()`, orders via `ioctl()`, bus payload rides inside the ioctl struct. The userland binary carries a byte-identical mirror struct ("Espejo del struct del kernel").

| `/dev` node | init | ioctl commands / struct (D-2 cap) | Status |
| :--- | :--- | :--- | :--- |
| `i2c0` | `I2C0, GPIO8/9` (~100 kHz) | `I2C_PROBE=0` (arg=addr, scalar), `I2C_READ=1`, `I2C_WRITE=2` · `I2cReq{addr,buf_ptr,len}` (≤64) | **Mechanism-only** (empty bus) |
| `spi0` | `SPI2, GPIO12/11/13` (10 MHz, mode 0) | `SPI_TRANSFER=0` · `SpiReq{buf_ptr,len}` (≤64, full-duplex in place) | **Mechanism-only** (MISO floats) |
| `sha0` | none (`SHA::steal()`) | `SHA256_CMD=0` · `ShaReq{in_ptr,in_len,out_ptr}` (in ≤512, out=32) | **Hardware-verified** (matches known vectors) |
| `power` | `LPWR` | `POWER_SLEEP=0`, `POWER_DEEP_SLEEP=1`, `POWER_REBOOT=2` (arg=seconds, **scalar**) | reboot ✅; **light-sleep hangs** (pre-existing); deep-sleep reboots on wake |
| `wlan0` | (net task) | `WLAN_NOP/CONNECT/DISCONNECT/SCAN` | ✅ verified |
| `ble0` | (net task) | `BLE_ADVERTISE=0` (enqueues) | ✅ from serial |

`flash.rs` (`esp-storage` NOR backend, `SECTOR_SIZE=4096`, 4-byte-aligned writes) has **no `/dev` node** — it is the kernel-internal block backend for espfs, OTA, and wifi_store. It reads capacity from **byte 3 of the image header at `0x0000`**, which is why `espflash.toml` must set `size = "16MB"` (§4). `uart.rs` is the hardware-verified live console: TX via `esp_println`, RX by reading the UART0 FIFO directly (non-blocking).

> **The kernel shell builtins bypass the D-1/ioctl path** and call the driver functions directly (`cmd_i2c`→`i2c::probe/read/write`, `cmd_spi`→`spi::transfer`, `cmd_sha256`→`crypto::sha256`). This is safe because a kernel-context caller returns `Ok` from `validate_user` (region-less → trusted). The **userland** binaries are the ones that exercise the full validated ioctl frontier.

### 3.7 Userland (ELF loader · libc · the shell · every /bin)

`userland/` is its own Cargo workspace (`libc` + `apps`, `panic=abort`, `opt-level="s"`, `lto=fat`). The target links with `-nostartfiles -force-frame-pointers -T.slots/espresso.x` — **the linker script is generated by `kernel/build.rs`** (no checked-in `user.x`, deliberately: a stale checked-in copy once silently won and caused a 64 KB drift).

**Route B — relocation without PIC (split-Harvard).** The ESP32-S3 executes external PSRAM code through a separate **instruction-bus** aperture from the data-bus view of the same RAM, and the LLVM Xtensa backend refuses PIC/PIE outright. So every binary is linked as an ordinary `ET_EXEC` at slot 0's fixed addresses, and the loader **relocates** it into a free slot by adding a per-slot bias. The ISA quirk that pays for it: Xtensa can't encode a 32-bit absolute in an instruction, so every far reference goes through the **literal pool** (data), and literals are what get patched. `kernel/build.rs` links each app with `ld --emit-relocs`, keeps only `R_XTENSA_32` data words, and appends a **fixup trailer** `<elf><fixups u32[]><count u32><magic "ESPF">` (found by seeking from EOF — no section headers parsed at runtime). The 879 `R_XTENSA_SLOT0_OP` PC-relative relocations across the set are skipped (a uniform text bias leaves them correct), so **no instruction is ever decoded** (cat needs 11 fixup words, sh needs 48).

- **The loader** (`fs/elf.rs`): `load_elf` → validate header (`\x7fELF`, `EM_XTENSA=94`) → read phdrs → `measure` text/data regions (split by i-bus range) → size-check ≤ `SLOT_SIZE` → `slot_alloc` → `place` (write each `PT_LOAD` through the **data alias**, zero-fill `.bss`, `apply_fixups` (RMW add of bias, bounds-checked — the trailer is unauthenticated), `sync_caches`). `write_argv` builds `[argc][argv ptrs][NULL][strings]` at the **top of the child's data slot** (grows down, fixed address); the child's `_start` gets the single blob pointer.
- **libc** (`userland/libc/src/lib.rs`): naked `_start` (`entry a1,32` rotates the register window to bring the blob pointer to `a2` — the ABI subtlety that a `#[naked]` `_start` without `entry` once broke), the raw `syscall` gate, **30 typed wrappers** (1:1 with the syscall table), a fixed **32 KB non-reclaiming bump allocator** (`dealloc` is a no-op), `print!`/`println!`, and `#[panic_handler] → exit(-1)`. Per-process envelope: 16 KB text slot + 16 KB data slot + 16 KB stack + 32 KB bump heap. Note: `O_RDONLY` is defined as `1` (not 0) throughout userland.
- **The userland shell** (`apps/src/bin/sh.rs`, ~24.7 KB) — the interactive serial console (spawned by `/bin/init`) and the `/etc/rc` interpreter. Quote-aware in-place tokenizer; **top-level `;`** sequencing (quote-aware); **N-stage pipelines** (`run_pipeline_n`, up to 8 stages, careful fd discipline so no stage waits on a pipe EOF that never comes); **`>`/`>>` redirection** per stage (a stage's own `>` beats the pipe); builtins **`cd`, `pwd`, `clear`, `sudo` (no-op prefix), `exit`, `help`**. The prompt shows the cwd (`/` → `~`), so `getcwd` is exercised every keystroke. `PATH` search: bare name → `/bin/<name>`, anything containing `/` used as-is (so `./hello` runs `/tmp/hello`). Scripts continue on a failed line (a broken `ls` in `/etc/rc` must not block reaching the console). *Known debt: no `&`/`&&`/`||` (background is R8); `print_help` is **stale** — it omits `;`, `>`/`>>`, quotes, and the cd/pwd/sudo builtins (H2).*

**Every `/bin` binary (32 total).** `kernel/build.rs`'s `APPS` list is authoritative; `install_userland` deploys them to `/bin` by content-diff on first boot. *(The old `userland/dist/*.elf` is a stale 10-file snapshot and is not the source of truth.)*

| Program | State |
| :--- | :--- |
| `init` | ✅ runs `sh /etc/rc` then respawns interactive `sh`; exits after 3 failed `sh` starts so the supervisor falls back. **Not PID 1 yet** (child of the kernel supervisor; real reparent is R9). |
| `sh` | ✅ the userland shell (above) |
| `cat [FILE...]` | ✅ honours argv; no args drains **stdin** (pipeline use) |
| `echo [-n] TEXT...` | ✅ honours argv |
| `ls [PATH...]` | ✅ honours argv; no args lists the inherited cwd (`.`) |
| `mkdir/touch/rm/write` | ✅ coreutils (`write FILE TEXT...` truncates + adds `\n`; `touch` = CREATE without TRUNC) |
| `wifi` | ✅ `status\|scan\|connect "SSID" [PASS]\|disconnect` over `/dev/wlan0` (read=state, ioctl=action). **Verified on board.** |
| `ip` | ✅ prints `wlan0: <ip> ssid "<ssid>" state <state>` |
| `nmcli` | ✅ nmcli-syntax shim → spawns `/bin/wifi` |
| `uptime/free/ps` | ✅ procfs viewers (`ps` = `/proc/tasks`, enumerates **all** tasks) |
| `smp/pms` | ✅ status-only sysfs viewers |
| `i2c/spi/sha256/power/ble/reboot/ota` | ✅ driver control via `/dev/*` ioctl (see §3.6). `sha256` is the strong differential; `ota` menu marks image INVALID to force rollback. |
| `sleep` | ✅ concurrency test — holds a slot long enough for a second instance to overlap |
| `cwdtest/ioctltest/badptr` | ✅ boundary self-tests (chdir/getcwd edges; ioctl D-1 EFAULT/EINVAL frontier; syscall pointer rejection) |
| `ping <IPv4>` | ✅ **ICMP Echo Request/Reply ping.** Uses native `smoltcp` ICMP raw sockets (`SOCK_RAW` / `IPPROTO_ICMP`), computes ICMP checksums, handles 1s probe timeouts, and reports sequence RTT and loss statistics. |
| `tcping <IPv4> [port]` | ✅ **TCP connectivity probe.** Processes `argv` (`<IPv4>`, optional `port`, default 80), measures TCP handshake RTT, and displays transmission/loss statistics. |
| `sntp [server_ip]` | ✅ **SNTP time synchronization client.** Processes `argv` for NTP server IP (defaults to `128.138.140.44`), configures `SO_RCVTIMEO` socket read timeout (2s), retries up to 3 times, updates system clock via `settimeofday`, and exits gracefully on failure without blocking indefinitely. |
| `httpd [port]` | ✅ **HTTP/1.1 web server.** Processes `argv` for port (defaults to 80), configures client socket read timeout (3s), dynamically serves `/proc/uptime` and `/proc/meminfo` on each request, sends valid HTTP/1.1 headers, and closes client sockets safely. |
| `netstat` | ✅ prints `/proc/net/sockets` verbatim |

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

---

## 4. Build & Flash

### 4.1 Prerequisites

```bash
cargo install espup --locked
cargo install espflash@3.3.0 --locked      # 3.x — NOT 4.x
espup install
. $HOME/export-esp.ps1                       # PowerShell   (Linux/macOS: source $HOME/export-esp.sh)
```

> **Use espflash 3.x, not 4.x.** This project targets `esp-hal 0.23`, whose image format predates the ESP-IDF App Descriptor that espflash **4.x** requires; with 4.x the bootloader rejects the image (`no bootable app`). Pin `espflash@3.3.0` until the project migrates to `esp-hal 1.0`. The toolchain is the Espressif `esp` channel (`rust-toolchain.toml`), installed via `espup` — not upstream Rust.

### 4.2 Configuration files (both load-bearing, neither fails loudly)

- **`espflash.toml`** — `partition_table = "partitions.csv"` and `[flash] size = "16MB"`, plus a `[[usb_device]]` filter (`1a86:55d3`, the CH343).
  - Without `size`, espflash writes `FlashSize::default()` = **4 MB** into byte 3 of the image header at `0x0000`; `esp-storage` derives capacity from exactly that byte, so everything above `0x400000` (the `fs` partition **and** `ota_0`) fails `OutOfBounds` → `EspFs::mount` returns `IoError` on a build that compiles perfectly.
  - Without `partition_table`, espflash flashes **its own default 3-entry table** (`nvs`/`phy_init`/`factory@0x10000`), and `otadata` (`0xF000`) lands on top of `phy_init`.
- **`partitions.csv`** — the 16 MB layout espflash parses directly. The kernel never reads the table; it addresses flash through `prelude::layout::*`, so a CSV/prelude mismatch compiles and boots but writes to the wrong partition. Optionally validate with `python tools/partition-gen/partition_gen.py`.

### 4.3 Set the boot Wi-Fi credentials (fallback only)

```bash
cp kernel/src/wifi_credentials.rs.example kernel/src/wifi_credentials.rs
#   edit WIFI_SSID / WIFI_PASSWORD (git-ignored, never committed)
```

These are a **fallback, not the boot network**. `wifi connect` (§5) saves the SSID + password to NVS at `0x9000`, and the boot path prefers a saved record — so on a board that has connected even once, editing this file and reflashing changes nothing. The boot log says which it used:

```
[net] using saved Wi-Fi credentials for 'Neighbor'      ← from flash; this file was ignored
[net] no saved Wi-Fi credentials; using compiled defaults
```

### 4.4 Build & flash

```bash
cargo build --release        # release is mandatory; build.rs also compiles + embeds the 32 userland ELFs
cargo run --release          # = espflash flash --monitor (from .cargo/config.toml); pick the port when prompted
```

`cargo run --release` runs `espflash flash --monitor`. To flash an already-built image explicitly on a fixed port:

```bash
espflash flash -p COM5 target/xtensa-esp32s3-none-elf/release/kernel
```

**Feature gates** (`kernel/Cargo.toml`; default = `["syscall-trap"]`):

| Feature | Default | Effect |
| :--- | :--- | :--- |
| `syscall-trap` | **on** | Real `syscall`-instruction trap (EXCCAUSE=1). Off → `invoke` calls the dispatcher directly (same ABI, no CPU trap). |
| `smp` | off | Starts the APP_CPU and schedules on both cores. |
| `pms` | off | Configures PMS / World-Controller memory protection + hardware stack guards. |
| `diag-ble-sync` | off | **Diagnostic mine.** Exposes ioctl `0xD1A6` running `start_advertising` **synchronously on the caller's stack** (the path that hangs). Structural invariant: can never reach `default`. |
| `diag-32k-stack` | off | **Diagnostic.** Bumps `DEFAULT_STACK_SIZE` 16 K→32 K (the B arm of the BLE stack-overflow A/B experiment). The compiler guarantees 16 K in default, so 32 K cannot ship. |

### 4.5 Expected serial output

```
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

Two lines matter. **`flash: 16 MB usable`** — if it says 4 MB, `espflash.toml` wasn't picked up and both `EspFs` and OTA are about to fail (the kernel prints a warning naming the unreachable partitions). **`/ mounted on flash (espfs)`** — `warning: EspFs::mount failed … using ramfs on /` means your files vanish on reboot. Also check the ESP-IDF bootloader's partition dump lists **six** entries with `factory` at `0x00020000` (three entries = espflash's default table, `partitions.csv` never reached the chip). The on-board LED blinks ~1 Hz as the heartbeat proof-of-multitasking. Monitor controls: **Ctrl+R** resets, **Ctrl+C** exits.

---

## 5. Command Reference

Two shells share almost the same command surface. **The kernel builtin shell** (reached over SSH today, and as the serial fallback) runs both builtins and `/bin` programs. **The userland `/bin/sh`** (the serial console today) runs `/bin` programs plus its own builtins (`cd/pwd/clear/sudo/exit/help`). Anything not a builtin is looked up in `/bin`.

### Kernel-shell builtins

`echo, help, clear, uptime, free, ps, reboot, ls, cd, pwd, cat, mkdir, touch, rm, write, i2c, spi, ota, syscalltest, smp, pms, power, sha256, ble, wifi, ip, nmcli, sudo` — plus any `/bin/<name>`.

| Command | Syntax | Description |
| :--- | :--- | :--- |
| `help` | `help` | Show the command list. |
| `clear` | `clear` | Clear the screen (ANSI). |
| `echo` | `echo [-n] TEXT...` | Print text; `-n` suppresses the trailing newline. |
| `uptime` | `uptime` | Time since boot. |
| `free` | `free` | Kernel heap usage **and** PSRAM exec slots in use (userland images aren't on the heap). |
| `ps` | `ps` | Kernel builtin prints only the **current** TID; `/bin/ps` (`/proc/tasks`) enumerates all tasks. |
| `reboot` | `reboot` | Software reset (`rst:0x3`). |
| `ls` | `ls [PATH]` | List a directory. **Dirs are suffixed `/`, devices `@`.** (`/bin/ls` does not yet add these suffixes — H1.) Does not list nested mount points (H4). |
| `cd`/`pwd` | `cd [PATH]` / `pwd` | Change/print the working directory. |
| `cat/mkdir/touch/rm/write` | as coreutils | `write FILE TEXT...` truncates. |
| `wifi` | `wifi status\|scan\|connect "SSID" [PASS]\|disconnect` | Runtime Wi-Fi management (§9). `scan` drops the link. |
| `ip` | `ip` | Show `wlan0` address, SSID, link state. |
| `nmcli` | `nmcli device status\|device wifi list\|device wifi connect "SSID" password "PASS"` | nmcli-compatible shim. |
| `sudo` | `sudo COMMAND ...` | No-op prefix (no privilege separation). |
| `i2c` | `i2c scan` · `i2c read ADDR_HEX LEN(1..64)` · `i2c write ADDR_HEX B0 [B1 ...]` | Master I2C on `/dev/i2c0`. |
| `spi` | `spi transfer B0 [B1 ...]` | Full-duplex SPI on `/dev/spi0`. |
| `sha256` | `sha256 [TEXT]` | Hardware SHA-256. |
| `power` | `power sleep [SECONDS]` · `power deep-sleep [SECONDS]` | Light (**hangs — pre-existing**) / deep sleep (reboots on wake). |
| `ble` | `ble status` · `ble advertise` | BLE status / advertise. **Do not run `ble advertise` over SSH** — the kernel builtin uses the synchronous mine (§8). |
| `smp`/`pms` | `smp` · `pms [world1]` | Feature-gated status. `pms world1` applies an experimental W^X (needs `--features pms`). |
| `ota` | `ota status\|set factory\|ota0\|rx\|apply` | A/B update (image received over TCP :3300). |
| `syscalltest` | `syscalltest` | Exercise the syscall ABI end to end. |

### Userland programs (`/bin`)

See the full 32-binary table in §3.7. Selected usage:

```
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

`free` reports the slot pool because a userland image lives in the reserved PSRAM region, not the heap — a slot leak would otherwise stay invisible until the 33rd launch failed.

---

## 6. The Shells — prompts, redirection, pipes, sessions

**Prompts** (cwd shown, `/` → `~` like bash):
- **Serial console** (`/bin/sh`): `EspressoOS:~$` — no user (no local login).
- **SSH** (kernel shell today): `youareme@EspressoOS:~$` — the authenticated `DEV_USER`.

**Redirection** — a `dup2` swap onto fd 1, so a command never learns it moved. Files get plain **LF** (the `\n`→`\r\n` translation belongs to the terminal `SessionChannel`, not to `echo`). `>` truncates, `>>` appends; `stderr` (fd 2) is not captured.

**Pipes** — every stage is launched at once and the shell waits for them all; a stage's own `>` beats the pipe. In the **kernel shell**, a pipeline stage must be a `/bin` program (a builtin runs inside the shell's own task and can't run concurrently — there is no `fork` to escape with), so `wifi | cat` fails with a clear error. The userland `sh` pipes `/bin` programs the same way.

**Sessions** — the serial console and each SSH session are independent processes: each owns a pid, an fd table with 0/1/2 bound to its own `SessionChannel`, and its own cwd (`cd` in one doesn't move the other; a new SSH session starts at `/`). Children inherit all of it through `clone_fd_table`.

**The `SessionChannel` convention** (`session.rs`) is load-bearing: `write` accepts as much as fits and never blocks — **`WouldBlock` (EAGAIN) = ring full, retry; `IoError` (EIO) = session gone, stop.** `read` returns `Ok(0)` = end of session, `WouldBlock` = nothing yet. ONLCR lives here. This is the exact non-blocking convention that `vfs/socket.rs` currently violates (§8, slice #14).

**Ending a session** — `exit`/`quit`/`logout`. Over SSH the shell task exits and the server sends a clean `CHANNEL_EOF`/`CHANNEL_CLOSE` (client prints `Connection to <ip> closed.`). On the console it reprints the banner and starts a fresh session (it deliberately does *not* end the task — the serial port is the board's only local way in).

---

## 7. The SP2→SP4 Mandate — status & decisions

The project is executing an autonomous **"total parity" mandate** (`docs/superpowers/plans/2026-07-17-mandato-sp2-sp4-paridad-total.md`), tracked in `DECISIONES.md`. **Definition of done** (all must be true *on hardware*): every README command works on serial AND SSH; SSH runs the userland `/bin/sh`; init is real PID 1 (reparent, `&` background, orphan reaping, no slot leaks); the kernel shell is out of the default build (behind `--features rescue-shell`); every `/bin` honours argv; every slice's hardware matrix is green; the README reflects reality. **"Green compile" is never done — done is verified on board.**

### Slice progress

| Slice | Scope | Status |
| :--- | :--- | :--- |
| **R0** | Stack watermark (`0xDEADBEEF` paint, `/proc/stacks`), global audit | ✅ done + HW-verified (idle 0, net 8144–8336, init 6680, `/bin/sh` 9160/16384) |
| **R1** | `/bin/mkdir,touch,rm,write` + `sudo` no-op | ✅ done + HW-verified |
| **R1.5** | Shell parity: quotes, N-stage pipes, `>`/`>>` | ✅ done + HW-verified |
| **R2** | Wi-Fi via `/dev/wlan0`+ioctl; `/bin/wifi,ip,nmcli,ioctltest`; `WLAN_NOP` | ✅ done + HW-verified (connect works, NVS persists, circular trap closed) |
| **R3** | `/proc/tasks` (+state,pid), meminfo slots, `/bin/uptime,free,ps`, `/sys/smp,pms` | ✅ done + HW-verified; `NET_STACK` 16 K→24 K |
| **R4** | i2c/spi via ioctl (D-1 struct, ≤64 D-2); `/bin/i2c,spi` | ✅ done + HW-verified — data path only vs an **empty bus** (needs SSD1306@0x3c to fully close) |
| **R5** | `/dev/sha0,power,ble0`; `/bin/sha256,power,ble,reboot` | ⚠️ **partial**. sha256 ✅✅, reboot ✅, ble status ✅; ble advertise D-4 fix applied+verified (scanner row pending); **`power sleep` = pre-existing platform hang** (diagnosed via live-oracle differential) |
| **slice #14** | Process-control usable: guard + `socket.rs`→WouldBlock + `/bin/kill` + `pid` column | **DECIDED (D), NOT yet implemented.** Ordered **before** R6 |
| **R6** | argv for `ping,sntp,netstat,httpd,sleep` | ✅ done + HW-verified (ping ICMP + argv; tcping, sntp 2s timeout + settimeofday, httpd port + 3s client timeout) |
| **R7** | SSH usable — replaced by the expanded R7.0–R7.6 plan | pending |
| **R8** | `&` background + reparent-to-init | pending |
| **R9** | init as real PID 1 | pending |
| **R10** | Retire the kernel shell from the default build | pending (blocked on R7.5 access-robustness) |
| **R11** | OTA in userland | pending |

**Net:** R0–R6 core landed and hardware-verified (R5 partial with two documented failures; R6 network binaries fully updated with ICMP, argv, and timeouts); **slice #14 decided but unwritten; R7–R11 pending.** Verified in code that slice #14 is not implemented — `vfs/socket.rs` still spins in `accept`/`read_at`/`write_at`, and `check_signals` has no user/kernel guard.

**Slice #14 — decision (D).** `socket.rs` (accept/read_at/write_at) returns **`WouldBlock`** instead of spin-with-yield — chosen as a *correction* (the non-blocking console convention is the project's; `sntp`/`httpd` were written against it; `socket.rs` is the lone deviator). Consequence: EINTR disappears (the spins move to userland; return → `check_signals` → `exit`), so the slice shrinks to **(1) guard in `check_signals` (deliver only on return to user mode) → (2) `socket.rs`→WouldBlock → (3) `/bin/kill` + `pid` column in `/proc/tasks`**. Order is a structural invariant: **no image ships `/bin/kill` without the guard.** `ping` will then need a userland poll loop (its single `connect` would fail instantly). `kill` urgency drops to "R7.3 Ctrl+C + hygiene". Full block-and-wake (`O_NONBLOCK` + a net_task waker) is deferred debt.

**R7 expanded ("SSH usable").** Root diagnosis: SSH *has more commands* (it runs the kernel shell) yet *feels worse* because the kernel shell got no ergonomics work in 8 slices while the userland `sh` got all of it. **Guiding principle: zero work on the kernel shell — it dies in R10; every improvement goes to userland `sh`, serial enjoys it immediately, SSH inherits it in R7.4.** Findings: H1 `/bin/ls` diverges (no `/`/`@` suffix); H2 `sh help` is stale; H3 `/bin/smp` format diverges; H4 mount points invisible in `ls /`; H5 `sh` has `read_line`, not a line editor; H6 SSH latency is a hypothesis, not measured; H7 access fragility (single session, dead client doesn't free the slot, exotic cipher). Tasks R7.0 (byte-for-byte oracle differential — last chance before R10) → R7.1/R7.2 (format parity + mount visibility) → R7.3 (interactivity + context-sensitive Ctrl+C) → R7.4 (SSH launches `/bin/sh`) → R7.5 (access robustness + a **hardware watchdog**, open question: what feeds it) → R7.6 (latency, only if measured). **R7.5 rows 1–4 are a blocking precondition for R10.**

### Design decisions D-1..D-12 + invariants

- **D-1** driver pattern: `/dev/<node>` + `ioctl(cmd, arg)` with a typed `{ptr,len}` struct; kernel validates the struct AND each inner pointer via `validate_user`.
- **D-2** limits as invariants: SSID ≤32, WPA pass ≤64, I2C 1..64, SPI ≤64, SHA in ≤512; mirrored kernel/libc.
- **D-3** state via `read()` (text), orders via `ioctl()`; bus data is the sanctioned exception (rides in the ioctl struct).
- **D-4** enqueue-and-return; the `net_task` executes and publishes (Wi-Fi scan, and the BLE advertise fix).
- **D-5** zero new syscalls in SP2 — the table stays frozen (R0–R5 added none).
- **D-6** self-sufficiency first (coreutils + wifi) closes the circular trap (dead network → no SSH → no `wifi` → reflash).
- **D-7** shell parity is part of 100% (quotes, N pipes, `>`/`>>`, `;`, multi-arg).
- **D-8** `/proc` for system info; `/proc/tasks` enumerates the whole table.
- **D-9** `ota` is last and never autonomous; every `ota apply` is a manual pause.
- **D-10** stack watermark before the first deep ioctl (R0).
- **D-11** kernel-shell retirement with a safety net: R10 removes it from default but keeps it behind `--features rescue-shell` for ≥1 cycle.
- **D-12** live lessons: `/etc/rc` (system) + `/etc/rc.local` (user, never seed-if-absent — the passwd lesson); never truncate silently; no I/O under a lock; big buffers off the stack (the 16 K lesson); every self-test knows its own answer.
- **Standalone invariants:** the **25%-free stack-margin rule**; the **oracle rule** (diff userland A/B against the live builtin before R7/R10 — inspection ≠ evidence); the **format-differential rule** (a command isn't ported until its output matches byte-for-byte or the divergence is registered); the **`help` rule** (no slice adds syntax/keys without updating `help`); **feature-gate-mines** as structural invariants (`diag-*` can never reach `default`/ship).

---

## 8. Known Issues & Technical Debt

Every item here is sourced from the code or the decision log — none is speculative.

**Kernel-side**
- **`socket.rs` spin → WouldBlock is pending** (slice #14 decision D, not yet written): `accept`/`read_at`/`write_at`/`sys_connect` TCP-wait still spin-with-yield inside the kernel, uninterruptible.
- **`kill`/signals exist but are UNEXPOSED.** `sys_kill` + `check_signals` (default-terminate on SIGINT/KILL/TERM) are wired, but there is no `/bin/kill`, no builtin, and it's not in `APPS`. So a spinning `httpd`/`ping` = reset today.
- **`check_signals` is a latent IRQ mine.** It runs on every interrupt with **no user/kernel guard** — inert now (`pending_signals` always 0), but the moment `kill` is exposed a tick mid-syscall could hijack a kernel frame or `exit()` over a half-done syscall → orphaned socket. The guard (deliver only on return to user mode) must land **first** — the slice #14 order invariant.
- **pid-vs-tid gap.** `ps`/`/proc/tasks` give `tid`; `sys_kill` wants `pid`; they are different namespaces. Even with `/bin/kill`, the user has no way to get the pid until slice #14 adds a `pid` column.
- **`power sleep` (light) hangs** — a pre-existing platform limitation (the kernel builtin hangs identically). `deep-sleep`/`reboot` are reliable.
- **espfs `compact()` latent hang** — a compacted image can be larger than the source and permanently wedge the fs (latent at current sizes; the verification net is in place — the "espfs compaction hang").
- **`readdir` doesn't list mount points** — `ls /` hides `/dev`, `/tmp`, `/proc`, `/sys` (H4).
- **SSH is single-session** — a killed client doesn't free the slot; recovery is only by serial reset (H7). A blocking precondition for R10.
- **No MPU, no SMP on default builds** — `pms`/`smp` are off, so no hardware stack-guard, no privilege separation, single-core only.
- **Priority is ignored** — `Task.priority` is stored but the policy is pure FIFO+affinity.
- **The Mutex is non-reentrant** — accidental nesting of SCHED/PROCESS_TABLE on one core wedges silently. Maintained by discipline, not the type system.

**Build / deploy**
- **Deploy is by diff** — `install_userland` restores a *deleted* binary next boot (it skips on size+content match). Excluding something from an image requires a build variant (feature-gate / removal from `APPS`), never `rm`. This is why the BLE mine was feature-gated, not deleted.

**Dead / stale / diagnostic code**
- `drivers/device.rs` registry is initialized but never queried (a second, divergent `Device` trait); `InodeKind::Symlink` is defined but no fs produces one; `littlefs` is an empty stub, not mounted; `crypto_smoke::smoke()` and `announce_host_key()` are never called; publickey SSH auth can't succeed (empty `authorized_key_blobs()`); `sh::print_help` is stale (H2); `userland/dist/*.elf` is a stale 10-file snapshot; the `diag-ble-sync`/`diag-32k-stack` experiment code is retained (feature-gated) as R7.5 watchdog fixtures.

**Security (dev-grade)**
- Compiled dev credentials in-tree (`DEV_USER="youareme"`, `DEV_PASSWORD` — the literal value lives in `kernel/src/drivers/ssh/config.rs`, intentionally **not reproduced in this public README**) and a fixed `HOST_KEY_SEED` (deterministic host key). `/etc/passwd`, if present, is plaintext and overrides the compiled credential (the kernel warns at boot). **Do not expose this server to the internet.**

---

## 9. Operational Notes

- **Opening the serial port resets the board (DTR/RTS).** `espflash monitor` and `System.IO.Ports.SerialPort.Open()` with `DtrEnable=$true`/`RtsEnable=$true` assert DTR/RTS = a hardware reset of the ESP32, which kills the non-persistent BLE advertise — the cause of every false-negative BLE scan. Proven on board: `.NET SerialPort.Open()` with **`DtrEnable=$false; RtsEnable=$false` does NOT reset** (no boot banner; uptime keeps climbing), and **`.Close()` doesn't reset either**. So a clean advertise window must happen in **one already-open serial session** without reopening. Coordinate COM5 — one reader at a time.
- **The board IP is discovered over serial with `ip`, never assumed.** The Wi-Fi fallback has silently changed subnets several times in a session (e.g. `192.168.100.77` ↔ `192.168.2.175`). Post-R10 (SSH the only remote path) a silent subnet change is a lockout vector, so the serial cable stays the last-resort recovery.
- **Wi-Fi credentials persist in NVS at `0x9000`.** `wifi connect` saves the SSID+password there (128-byte `EWC1` record) and it becomes the boot network, surviving a kernel reflash. `phy_init` at `0xF000` is deliberately untouched. Clear it by erasing that sector (next boot falls back to the compiled defaults).
- **`wifi scan` drops the network** — it disconnects, scans, then reconnects and re-acquires DHCP, so **an SSH session over Wi-Fi dies on it**. Run it from the serial console.
- **Never run `ble advertise` over SSH** — the kernel-shell builtin uses the synchronous HCI path (the original mine). Over SSH, use `ble status` only; arm advertising from the serial console (`/bin/ble advertise`, the D-4 async path).
- **A working SSH oracle method** (paramiko fails — no `chacha20-poly1305@openssh`; plink hangs on the host key): Windows `ssh.exe` + `SSH_ASKPASS` + `-o StrictHostKeyChecking=accept-new`, and **reset the board (RTS pulse) before each connection** because SSH is single-session and a killed client doesn't free the slot.

You can also poke the raw network path with the TCP echo server on port 2323: `printf 'hello\n' | nc <board-ip> 2323`.

---

## Repository Structure

```
EspressoOS/
├── .cargo/config.toml       # Xtensa target + `cargo run` = espflash flash --monitor
├── rust-toolchain.toml      # channel = "esp"
├── bootloader/              # 2nd-stage bootloader (stub crate, excluded from the workspace)
├── kernel/                  # Kernel crate (package: espressoos-kernel, binary: kernel)
│   ├── build.rs             # Compiles the 32 userland ELFs, extracts fixups, emits the linker script + USERLAND_BINARIES
│   ├── src/
│   │   ├── arch/xtensa/     # context switch (Model B), vectors, SYSTIMER, non-reentrant Mutex
│   │   ├── drivers/         # gpio, uart, i2c, spi, crypto, power, ble, wifi, wifi_store, ssh/, device(dead)
│   │   ├── fs/              # espfs, ramfs, procfs, sysfs, littlefs(stub), elf.rs (loader)
│   │   ├── mm/              # heap, psram_exec (slot pool), mpu (pms)
│   │   ├── scheduler/       # task, policy (FIFO), process (pid/signals), core_sync (SMP)
│   │   ├── shell/           # commands/, parser  (kernel builtin shell — the oracle)
│   │   ├── session.rs       # SessionChannel (Uart|Ssh) + SessionConsole inode
│   │   ├── syscall/         # table (0..29), handler, trap
│   │   ├── vfs/             # inode, file, mount, devfs, pipe, socket
│   │   ├── ota/             # A/B slots + otadata
│   │   ├── wifi_credentials.rs   # GIT-IGNORED boot Wi-Fi fallback
│   │   └── main.rs          # boot sequencer
│   └── Cargo.toml
├── userland/                # no_std libc + /bin programs (ELF, run from PSRAM)
│   ├── libc/                # _start, raw syscall, 30 typed wrappers, 32 KB bump heap
│   └── apps/src/bin/        # 32 programs (sh, coreutils, wifi/net, /dev drivers, self-tests)
├── tools/                   # partition-gen, mkimage, build-userland.ps1, tests/ (logic_tests.py)
├── docs/superpowers/plans/  # the SP2→SP4 mandate + DECISIONES.md
├── espflash.toml            # flash size + partition table — both load-bearing (§4)
├── partitions.csv           # 16 MB flash layout (espflash reads this directly)
└── README.md                # this file
```

---

## Memory Map & Partition Table

The **16 MB** flash is laid out for A/B updates (`prelude::layout` is authoritative; `partitions.csv` is what espflash flashes):

```
0x000000 ┤ Bootloader (2nd stage, ROM + esp-hal runtime)
0x008000 ┤ Partition table
0x009000 ┤ NVS (24 KB)          ← Wi-Fi creds persist here (EWC1 @ 0x9000)
0x00F000 ┤ otadata (A/B boot control, 8 KB)
0x020000 ┤ factory app — Slot A (primary kernel, 4 MB)
0x420000 ┤ ota_0 app   — Slot B (secondary kernel, 4 MB)
0x820000 ┤ filesystem (EspFs, ~7.8 MB)
0xFF0000 ┤ coredump (64 KB)
```

The two are **not** cross-checked at build time and the kernel never reads the partition table — it addresses flash straight through `layout::*`. A `prelude.rs` that disagrees with `partitions.csv` compiles, boots, and silently writes into the wrong partition. Keep them in sync by hand.

**RAM:** 512 KB internal SRAM (a **128 KB** static kernel-heap region + esp-wifi `Internal` allocations) plus **8 MB PSRAM** (~7 MB general heap + the reserved 1 MB executable userland region: instruction alias `0x42800000`, data linked at `0x3c170000`).

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
