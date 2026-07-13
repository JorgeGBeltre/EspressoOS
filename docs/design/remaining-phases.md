# EspressoOS — Design: Completing Phases 3–9

Status: approved design (2026-07-13). This document specifies the completion of every
remaining roadmap phase. It is the source of truth for the implementation.

## Constraints and reality

- **No hardware-in-the-loop during authoring.** Flashing and on-device validation are done
  separately. Every phase is delivered *compile-clean* (`cargo check` against
  `xtensa-esp32s3-none-elf` with `build-std`) and *logic-tested* (Python harness under
  `tools/tests/`), then validated on the board and iterated from serial / `ssh -vvv` output.
- **Do not destabilize the known-good image.** The default build boots and serves WiFi + SSH.
  High-risk phases (SMP, PMS) are **feature-gated and opt-in** so the default build is unchanged.
- **No userland / privilege rings exist yet.** Syscalls (P6) and memory protection (P8) are
  therefore *mechanism* work: they install and exercise the hardware path, but there is no
  unprivileged code to police yet. This is stated as a documented limitation, not a defect.

## Key decisions

1. **Persistence (P4) is a pure-Rust filesystem.** No C `littlefs2-sys` (bindgen/cc for Xtensa
   inside a `no_std` `build-std` kernel is the single most fragile option). We implement
   **`EspFs`**, a small log-structured, wear-leveled filesystem in Rust that implements the
   existing `vfs::inode` traits and persists to `drivers::flash`. The `fs/littlefs` stub stays
   as documented dead code / future alternative.
2. **SMP (P9) and PMS (P8) are behind cargo features `smp` and `pms`.** Implemented fully, but
   the default feature set excludes them.

## Verification strategy (per phase)

1. `cargo check` clean (default features) — and additionally `--features smp` / `--features pms`
   for those phases.
2. Add/extend Python logic tests in `tools/tests/` for any non-trivial pure logic (path/geometry,
   FS record encoding, OTA already covered, wear-leveling selection, SMP run-queue policy).
3. Hardware checklist appended per phase (what serial output proves it works).

---

## Phase 3 — Bus drivers (I2C / SPI)  [risk: low]

**Goal:** the existing esp-hal wrappers become live, safe, and reachable from userspace-ish
(`/dev` + shell).

**Problems in current code:**
- `i2c::init` / `spi::init` are never called.
- Both call `Peripherals::steal()`, which is unsound after `esp_hal::init` already consumed
  `Peripherals` in `main`. Must instead receive their peripheral handles from `main`.
- No `/dev` node; no way to use them.

**Design:**
- Refactor `drivers/i2c.rs` and `drivers/spi.rs` to `provide_peripherals(...)` /
  `init(...)` that take the specific peripheral + pins from `main` (mirror the existing
  `wifi::provide_peripherals` pattern) instead of `steal()`.
- Add devfs devices: `/dev/i2c0` and `/dev/spi0` implementing `vfs::devfs::Device`
  (`ioctl` selects address/mode; `write`/`read` do bus transactions). Register in devfs init.
- Add shell commands `i2c` (scan / read / write) and `spi` (transfer) for interactive use.
- Pins kept configurable via constants; document defaults (I2C SDA=GPIO8/SCL=GPIO9,
  SPI SCK=12/MOSI=11/MISO=13) and that they must not collide with flash/PSRAM/USB.

**Tests:** logic test for the `ioctl` command encoding and the i2c-scan address iteration.

**Hardware check:** `i2c scan` lists ACKing addresses; with nothing attached it returns an empty
set without hanging.

---

## Phase 4 — Persistent filesystem `EspFs` on `/`  [risk: medium]

**Goal:** files created in the shell survive a reboot, mounted at `/` on the `fs` flash region
(`layout::FS_OFFSET = 0x820000`, `FS_SIZE = 0x7D0000`).

**Design — `kernel/src/fs/espfs/`:**
- A small **log-structured** filesystem over `drivers::flash` (4096-byte sectors).
- On-flash layout: a superblock (magic, version, geometry, generation counter) written to one of
  two superblock sectors (ping-pong for power-fail safety); the remainder is a sequence of
  append-only **records**. Each record = `{crc32, type, inode, seq, payload}`; types:
  `DirEntry`, `FileData(offset,bytes)`, `Truncate`, `Unlink`, `Mkdir`. Mount replays the log to
  build an in-RAM index (`BTreeMap<Ino, Node>` where a `Node` for a file holds an extent list of
  `(flash_offset, len)` spans; a dir holds name→ino). Reads pull bytes from flash via the extent
  list; writes append a new `FileData` record and update the extent list.
- **Wear leveling / compaction:** append until the region is ~full, then compact live data into a
  fresh region pass (garbage-collect superseded records), bumping the generation counter. Erase
  is per-sector. `block_cycles`-style leveling comes free from append-everywhere + compaction.
- **Power-fail safety:** a record is only "committed" once its CRC matches on replay; a torn tail
  record is ignored. Superblock ping-pong means an interrupted compaction falls back to the prior
  generation.
- Implements `vfs::inode::{FileSystem, Inode}` exactly like `ramfs`, so `vfs::mount("/", EspFs)`
  drops in. `format()` writes a fresh superblock over an erased region.
- `main.rs`: mount `EspFs` at `/` when the region has a valid superblock, else `format()` then
  mount; keep `RamFs` on `/tmp`. If mount fails, fall back to `RamFs` on `/` (never fail to boot).

**Tests (Python port of the pure logic):** record encode/decode + CRC; log replay producing the
right final tree given a record sequence including supersedes/unlinks/torn tail; extent-list read
assembly; compaction preserving live data.

**Hardware check:** `write /foo hello` → `cat /foo` → reboot → `cat /foo` still `hello`;
`ls /` persists dirs.

**Documented gap:** not crash-consistent to the strength of a journaled fs with fsync barriers on
every op; it is power-fail *safe* (no corruption of committed data) via CRC + superblock ping-pong.

---

## Phase 5 — OTA A/B, wired  [risk: medium]

**Goal:** apply a new app image to the inactive slot and mark it bootable, from the running OS.

**Design:**
- `ota/mod.rs` already writes the inactive slot + sets otadata. Add:
  - shell command `ota status` (show active slot, seqs, validity via `partition::read_otadata`),
    `ota recv` (receive an image over the SSH channel / a TCP port and stream it through
    `OtaUpdate::write`), `ota commit` / `ota abort`.
  - image header validation (magic `0xE9`, length sanity vs slot size) — already partially present.
- **Documented gap (important):** the stock bootloader flashed by `espflash` does **not** consult
  `otadata` to pick factory vs ota_0. So writing otadata does not yet change what boots. True A/B
  boot-switching requires the project's own 2nd-stage bootloader (the `bootloader/` crate,
  currently excluded from the workspace). This phase delivers the *image transport + otadata
  update + validation*; boot-switching is tracked as the bootloader work item.

**Tests:** OTA slot selection is already covered in `tools/tests`; add a test for the image-header
sanity check and the `ota recv` chunk state machine.

**Hardware check:** `ota status` prints the correct active slot and seqs; `ota recv` of a valid
image reports the right byte count and a `finish` that updates otadata (verified by re-reading
`ota status`), without touching the running slot.

---

## Phase 6 — Syscall exception path  [risk: medium]

**Goal:** the `syscall` instruction traps into the kernel and is routed to `syscall::dispatch`.

**Design:**
- Install an Xtensa **user exception** handler that decodes `EXCCAUSE`; for
  `EXCCAUSE == 1` (SYSCALL) read the syscall number/args from the saved a2..a7, call
  `syscall::dispatch(num, &args)`, write the return value back to a2, and advance `EPC1` past the
  `syscall` instruction before `rfe`. Use the `xtensa-lx-rt` `#[exception]` hook where available so
  we cooperate with `esp-backtrace` instead of stealing its vector (other EXCCAUSE values chain to
  the existing panic/backtrace handler).
- Add a tiny `syscall::invoke(num, args)` inline-asm wrapper (emits the `syscall` instruction) and
  a `syscalltest` shell command that round-trips (e.g. `SYS_UptimeMs`, `SYS_Write` to console) to
  prove the trap works.
- **Documented limitation:** no privilege separation yet, so this exercises the trap/return
  mechanism; user-pointer validation in `handler.rs` remains best-effort until PMS/rings land.

**Tests:** dispatch-table number mapping (Python) already trivial; add argument-marshalling logic
test.

**Hardware check:** `syscalltest` prints uptime obtained *through* the trap and echoes a string
written via `SYS_Write`, and normal operation (WiFi/SSH, panic backtrace) is unaffected.

---

## Phase 8 — PMS memory protection  [risk: high, feature `pms`]

**Goal:** configure ESP32-S3 PMS / World Controller to mark kernel regions and trap stray access.

**Design (behind `--features pms`):**
- `mm/mpu.rs` grows a real API: `configure()` sets PMS "permission control" registers for the
  main memory split we care about (e.g. protect the exception vectors / `.rodata` from writes,
  mark a guard region around task stacks). Region programming is conservative: start by making a
  *known-unused* guard region non-writable and verifying a deliberate access faults, before
  tightening real regions.
- Because a wrong PMS setting hangs boot, `configure()` is only called when the `pms` feature is
  on, and `main` logs each step so a hang localizes to the offending register write.
- Reference the ESP32-S3 TRM PMS/World-Controller register block; encode register offsets as
  documented constants with the TRM section noted.

**Tests:** register-value computation (region base/size/permission encoding) as pure-logic Python.

**Hardware check (opt-in build):** with `--features pms`, boot still reaches the shell; a
deliberate write to a protected guard region triggers the fault handler with the expected cause.

---

## Phase 9 — SMP dual-core  [risk: high, feature `smp`]

**Goal:** bring up APP_CPU and schedule tasks across both LX7 cores.

**Design (behind `--features smp`):**
- `scheduler/core_sync.rs`: `start_secondary_core()` uses `esp_hal::system::CpuControl` to start
  APP_CPU on a dedicated stack running a per-core entry that installs its own SYSTIMER-equivalent
  tick and calls the scheduler.
- **Scheduler multicore refactor (feature-gated):** replace the single `current`/`idle` with
  per-core state (`current[NCORES]`, `idle[NCORES]`), keep one global `ready` queue guarded by a
  spinlock that does **not** rely on disabling the *other* core's interrupts; add
  `raw_core_id()`. `switch_to` already uses per-core windowed registers/`epc1`, so it is
  core-safe; only the shared scheduler bookkeeping needs the lock discipline. Add a cross-core
  reschedule IPI (or, minimally, let each core's own tick pick from the shared ready queue).
- Core affinity: default any-core; pin the `net`/SSH task to PRO_CPU (core 0) initially to avoid
  disturbing the validated WiFi path.
- The single-core build path is completely unchanged when `smp` is off.

**Tests:** run-queue selection with two cores pulling from the shared queue (no double-dispatch of
one Tid); Python model of the pick/re-queue invariant.

**Hardware check (opt-in build):** with `--features smp`, both cores report in via a per-core
heartbeat with distinct core ids; the WiFi/SSH task keeps working; no deadlock across a few
minutes of ticks.

---

## What actually landed (2026-07-13)

All phases below compile clean for `xtensa-esp32s3-none-elf` and every feature combination
links in `--release`; 104/104 Python logic tests pass. On-device validation is still pending.

- **P3** — `i2c.rs`/`spi.rs` refactored off `Peripherals::steal()` to generic `init()` taking
  peripherals from `main`; `/dev/i2c0` + `/dev/spi0` devfs nodes; `i2c` / `spi` shell commands.
- **P4** — `fs/espfs/` (`wire.rs` + `mod.rs`): pure-Rust log-structured FS, two-half ping-pong
  compaction, CRC'd records, superblock redundancy; mounted at `/` with RamFs fallback;
  `tools/tests/espfs_tests.py` (11 tests).
- **P5** — `ota status` / `ota set` + `otadata_entries()` + `validate_header()`. **Deliberately no
  WiFi image receiver** (multi-MB flash writes with the radio active would disable interrupts for
  too long and likely crash esp-wifi). Boot-switching still needs the custom bootloader.
- **P6** — `syscall::invoke` software gate + `syscalltest`; real trap in `syscall/trap.rs` behind
  `--features syscall-trap` (overrides weak `__exception`, delegates non-syscall causes to
  esp-backtrace's `__user_exception`). Confirmed the override links (no duplicate symbol).
- **P8** — `mm/mpu.rs` behind `--features pms`: DRAM0 violation monitor (safe, boot-enabled) +
  `report()` + `protect_world1()` (opt-in via `pms world1`; safe because the kernel is World-0).
  Uses the typed `SENSITIVE` PAC registers.
- **P9** — `scheduler/core_sync.rs` behind `--features smp`: starts APP_CPU via `CpuControl`,
  per-core heartbeat + shared `AtomicU32` counter; `smp` shell command. Scheduler kept single-core;
  full cross-core scheduling deferred.

Notable constraints discovered: no `AtomicU64` on this 32-bit target; esp-backtrace owns
`__user_exception` (not `__exception`), which is what makes the trap coexistence clean;
`FieldWriter::bits()` is `unsafe`; the `sensitive` register-block type is private (use `&*PTR`).

## Delivery order

Safe/high-value first, each gated on `cargo check`:
**P3 → P4 → P5 → P6**, then the feature-gated **P9 → P8**. After each, a hardware checklist for the
user to validate on the board before we move on. A final consistency pass updates the README
roadmap statuses to match what actually landed.
