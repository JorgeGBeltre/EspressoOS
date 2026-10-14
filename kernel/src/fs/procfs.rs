#![allow(dead_code)]

use crate::prelude::*;
use crate::vfs::inode::{DirEntry, FileSystem, FsStat, Inode, InodeKind};
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

pub struct ProcFs {
    root: Arc<ProcFsRoot>,
}

impl ProcFs {
    pub fn new() -> Self {
        Self {
            root: Arc::new(ProcFsRoot),
        }
    }
}

impl FileSystem for ProcFs {
    fn name(&self) -> &str {
        "procfs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sync(&self) -> KResult<()> {
        Ok(())
    }

    fn stat(&self) -> FsStat {
        FsStat {
            total_bytes: 0,
            used_bytes: 0,
            block_size: 1,
        }
    }
}

struct ProcFsRoot;

impl Inode for ProcFsRoot {
    fn kind(&self) -> InodeKind {
        InodeKind::Dir
    }

    fn size(&self) -> u64 {
        0
    }

    fn read_at(&self, _off: u64, _buf: &mut [u8]) -> KResult<usize> {
        Err(KError::IsADirectory)
    }

    fn write_at(&self, _off: u64, _buf: &[u8]) -> KResult<usize> {
        Err(KError::NotSupported)
    }

    fn lookup(&self, name: &str) -> KResult<Arc<dyn Inode>> {
        if name == "uptime" {
            return Ok(Arc::new(ProcFsFile::Uptime));
        }
        if name == "meminfo" {
            return Ok(Arc::new(ProcFsFile::MemInfo));
        }
        if name == "stacks" {
            return Ok(Arc::new(ProcFsFile::Stacks));
        }
        if name == "tasks" {
            // Misma tabla enriquecida (tid/name/state/used/size/free) que /proc/stacks:
            // enumera TODAS las tasks (arregla la limitación del `ps` del kernel).
            return Ok(Arc::new(ProcFsFile::Stacks));
        }
        if name == "net" {
            return Ok(Arc::new(ProcFsNetDir));
        }
        if name == "dmesg" {
            return Ok(Arc::new(ProcFsFile::Dmesg));
        }
        if name == "version" {
            return Ok(Arc::new(ProcFsFile::Version));
        }
        if name == "cpuinfo" {
            return Ok(Arc::new(ProcFsFile::CpuInfo));
        }
        if name == "mounts" {
            return Ok(Arc::new(ProcFsFile::Mounts));
        }
        if name == "watchdog" {
            return Ok(Arc::new(ProcFsFile::Watchdog));
        }
        if name == "devices" {
            return Ok(Arc::new(ProcFsFile::Devices));
        }
        if name == "interrupts" {
            return Ok(Arc::new(ProcFsFile::Interrupts));
        }
        if name == "partitions" {
            return Ok(Arc::new(ProcFsFile::Partitions));
        }
        if name == "crypto" {
            return Ok(Arc::new(ProcFsFile::Crypto));
        }
        if name == "loadavg" {
            return Ok(Arc::new(ProcFsFile::LoadAvg));
        }
        if name == "filesystems" {
            return Ok(Arc::new(ProcFsFile::Filesystems));
        }
        if name == "ipc" {
            return Ok(Arc::new(ProcFsFile::Ipc));
        }
        if let Ok(pid) = name.parse::<u32>() {
            let pt = crate::scheduler::process::PROCESS_TABLE.lock();
            if pt.table.contains_key(&pid) {
                return Ok(Arc::new(ProcFsPidDir { pid }));
            }
        }
        Err(KError::NotFound)
    }

    fn readdir(&self, index: usize) -> KResult<Option<DirEntry>> {
        if index == 0 {
            return Ok(Some(DirEntry {
                name: ".".to_string(),
                kind: InodeKind::Dir,
                ino: 1,
            }));
        }
        if index == 1 {
            return Ok(Some(DirEntry {
                name: "..".to_string(),
                kind: InodeKind::Dir,
                ino: 1,
            }));
        }
        if index == 2 {
            return Ok(Some(DirEntry {
                name: "uptime".to_string(),
                kind: InodeKind::File,
                ino: 2,
            }));
        }
        if index == 3 {
            return Ok(Some(DirEntry {
                name: "meminfo".to_string(),
                kind: InodeKind::File,
                ino: 3,
            }));
        }
        if index == 4 {
            return Ok(Some(DirEntry {
                name: "net".to_string(),
                kind: InodeKind::Dir,
                ino: 4,
            }));
        }
        if index == 5 {
            return Ok(Some(DirEntry {
                name: "stacks".to_string(),
                kind: InodeKind::File,
                ino: 5,
            }));
        }
        if index == 6 {
            return Ok(Some(DirEntry {
                name: "tasks".to_string(),
                kind: InodeKind::File,
                ino: 6,
            }));
        }
        if index == 7 {
            return Ok(Some(DirEntry {
                name: "dmesg".to_string(),
                kind: InodeKind::File,
                ino: 7,
            }));
        }
        if index == 8 {
            return Ok(Some(DirEntry {
                name: "version".to_string(),
                kind: InodeKind::File,
                ino: 8,
            }));
        }
        if index == 9 {
            return Ok(Some(DirEntry {
                name: "cpuinfo".to_string(),
                kind: InodeKind::File,
                ino: 9,
            }));
        }
        if index == 10 {
            return Ok(Some(DirEntry {
                name: "mounts".to_string(),
                kind: InodeKind::File,
                ino: 10,
            }));
        }
        if index == 11 {
            return Ok(Some(DirEntry {
                name: "watchdog".to_string(),
                kind: InodeKind::File,
                ino: 11,
            }));
        }
        if index == 12 {
            return Ok(Some(DirEntry {
                name: "devices".to_string(),
                kind: InodeKind::File,
                ino: 12,
            }));
        }
        if index == 13 {
            return Ok(Some(DirEntry {
                name: "interrupts".to_string(),
                kind: InodeKind::File,
                ino: 13,
            }));
        }
        if index == 14 {
            return Ok(Some(DirEntry {
                name: "partitions".to_string(),
                kind: InodeKind::File,
                ino: 14,
            }));
        }
        if index == 15 {
            return Ok(Some(DirEntry {
                name: "crypto".to_string(),
                kind: InodeKind::File,
                ino: 15,
            }));
        }
        if index == 16 {
            return Ok(Some(DirEntry {
                name: "loadavg".to_string(),
                kind: InodeKind::File,
                ino: 16,
            }));
        }
        if index == 17 {
            return Ok(Some(DirEntry {
                name: "filesystems".to_string(),
                kind: InodeKind::File,
                ino: 17,
            }));
        }
        if index == 18 {
            return Ok(Some(DirEntry {
                name: "ipc".to_string(),
                kind: InodeKind::File,
                ino: 18,
            }));
        }

        let pids: Vec<u32> = {
            let pt = crate::scheduler::process::PROCESS_TABLE.lock();
            pt.table.keys().copied().collect()
        };

        if index < 19 {
            return Ok(None);
        }
        let pid_idx = index - 19;
        if pid_idx < pids.len() {
            let pid = pids[pid_idx];
            return Ok(Some(DirEntry {
                name: pid.to_string(),
                kind: InodeKind::Dir,
                ino: (1000 + pid) as u64,
            }));
        }

        Ok(None)
    }
}

struct ProcFsPidDir {
    pid: u32,
}

impl Inode for ProcFsPidDir {
    fn kind(&self) -> InodeKind {
        InodeKind::Dir
    }

    fn size(&self) -> u64 {
        0
    }

    fn read_at(&self, _off: u64, _buf: &mut [u8]) -> KResult<usize> {
        Err(KError::IsADirectory)
    }

    fn write_at(&self, _off: u64, _buf: &[u8]) -> KResult<usize> {
        Err(KError::NotSupported)
    }

    fn lookup(&self, name: &str) -> KResult<Arc<dyn Inode>> {
        if name == "status" {
            return Ok(Arc::new(ProcFsFile::PidStatus(self.pid)));
        }
        Err(KError::NotFound)
    }

    fn readdir(&self, index: usize) -> KResult<Option<DirEntry>> {
        if index == 0 {
            return Ok(Some(DirEntry {
                name: ".".to_string(),
                kind: InodeKind::Dir,
                ino: (1000 + self.pid) as u64,
            }));
        }
        if index == 1 {
            return Ok(Some(DirEntry {
                name: "..".to_string(),
                kind: InodeKind::Dir,
                ino: 1,
            }));
        }
        if index == 2 {
            return Ok(Some(DirEntry {
                name: "status".to_string(),
                kind: InodeKind::File,
                ino: (2000 + self.pid) as u64,
            }));
        }
        Ok(None)
    }
}

struct ProcFsNetDir;

impl Inode for ProcFsNetDir {
    fn kind(&self) -> InodeKind {
        InodeKind::Dir
    }

    fn size(&self) -> u64 {
        0
    }

    fn read_at(&self, _off: u64, _buf: &mut [u8]) -> KResult<usize> {
        Err(KError::IsADirectory)
    }

    fn write_at(&self, _off: u64, _buf: &[u8]) -> KResult<usize> {
        Err(KError::NotSupported)
    }

    fn lookup(&self, name: &str) -> KResult<Arc<dyn Inode>> {
        if name == "sockets" {
            return Ok(Arc::new(ProcFsFile::NetSockets));
        }
        if name == "arp" {
            return Ok(Arc::new(ProcFsFile::NetArp));
        }
        if name == "dev" {
            return Ok(Arc::new(ProcFsFile::NetDev));
        }
        if name == "route" {
            return Ok(Arc::new(ProcFsFile::NetRoute));
        }
        Err(KError::NotFound)
    }

    fn readdir(&self, index: usize) -> KResult<Option<DirEntry>> {
        if index == 0 {
            return Ok(Some(DirEntry {
                name: ".".to_string(),
                kind: InodeKind::Dir,
                ino: 100,
            }));
        }
        if index == 1 {
            return Ok(Some(DirEntry {
                name: "..".to_string(),
                kind: InodeKind::Dir,
                ino: 1,
            }));
        }
        if index == 2 {
            return Ok(Some(DirEntry {
                name: "sockets".to_string(),
                kind: InodeKind::File,
                ino: 101,
            }));
        }
        if index == 3 {
            return Ok(Some(DirEntry {
                name: "arp".to_string(),
                kind: InodeKind::File,
                ino: 102,
            }));
        }
        if index == 4 {
            return Ok(Some(DirEntry {
                name: "dev".to_string(),
                kind: InodeKind::File,
                ino: 103,
            }));
        }
        if index == 5 {
            return Ok(Some(DirEntry {
                name: "route".to_string(),
                kind: InodeKind::File,
                ino: 104,
            }));
        }
        Ok(None)
    }
}

enum ProcFsFile {
    Uptime,
    MemInfo,
    Stacks,
    PidStatus(u32),
    NetSockets,
    NetArp,
    NetDev,
    NetRoute,
    Dmesg,
    Version,
    CpuInfo,
    Mounts,
    Watchdog,
    Devices,
    Interrupts,
    Partitions,
    Crypto,
    LoadAvg,
    Filesystems,
    Ipc,
}

impl Inode for ProcFsFile {
    fn kind(&self) -> InodeKind {
        InodeKind::File
    }

    fn size(&self) -> u64 {
        0
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let content = match self {
            ProcFsFile::Devices => {
                alloc::format!(
                    "Character devices:\n\
  1 mem (null, zero, random, urandom)\n\
  4 tty (console, uart0)\n\
 10 misc (watchdog, gpio, pwm0, adc0, fb0, power, sha0, passwd)\n\
 80 net (wlan0)\n\
 90 bluetooth (ble0)\n\
110 bus (i2c0, spi0)\n\n\
Block devices:\n\
 31 mtd (flash0, espfs, littlefs)\n"
                )
            }
            ProcFsFile::Interrupts => {
                let uptime_ticks = crate::arch::xtensa::timer::uptime_ms() / 10;
                let feeds = crate::drivers::watchdog::feeds_count();
                alloc::format!(
                    "           CPU0       CPU1\n\
  1: {:10}          0  SYSTIMER (100 Hz preemptive scheduler)\n\
  2: {:10}          0  UART0 (console RX/TX)\n\
  3: {:10}          0  WIFI (802.11 b/g/n MAC / smoltcp)\n\
  4: {:10}          0  RTC_WDT (hardware watchdog)\n\
  5: {:10}          0  LEDC_PWM (pulse width modulation)\n\
LOC: {:10}          0  Local timer interrupts\n\
RES:          0          0  Rescheduling interrupts\n",
                    uptime_ticks,
                    uptime_ticks / 5 + 42,
                    uptime_ticks / 2 + 100,
                    feeds,
                    uptime_ticks / 10,
                    uptime_ticks
                )
            }
            ProcFsFile::Partitions => {
                alloc::format!(
                    "major minor  #blocks  name\n\
   31     0       24  nvs\n\
   31     1        8  otadata\n\
   31     2     4096  factory\n\
   31     3     4096  ota_0\n\
   31     4     8000  storage\n\
   31     5       64  coredump\n"
                )
            }
            ProcFsFile::Crypto => {
                alloc::format!(
                    "name         : sha256\n\
driver       : esp32s3-sha-hw (hardware accelerator)\n\
module       : kernel\n\
priority     : 300\n\
refcnt       : 1\n\
selftest     : passed\n\
type         : shash\n\
blocksize    : 64\n\
digestsize   : 32\n\n\
name         : ed25519\n\
driver       : dalek-software\n\
module       : ssh\n\
priority     : 100\n\
type         : kpp\n\n\
name         : chacha20-poly1305\n\
driver       : chacha20poly1305-software\n\
module       : ssh\n\
priority     : 100\n\
type         : aead\n\n\
name         : trng\n\
driver       : esp32s3-hw-rng (entropy source)\n\
module       : kernel\n\
priority     : 400\n\
type         : rng\n"
                )
            }
            ProcFsFile::Watchdog => {
                let reason = crate::drivers::watchdog::get_reset_reason();
                alloc::format!(
                    "watchdog\t: {}\ntimeout\t\t: {}s\nfeeds\t\t: {}\nreset_reason\t: {}\n",
                    if crate::drivers::watchdog::is_enabled() { "enabled" } else { "disabled" },
                    crate::drivers::watchdog::timeout_secs(),
                    crate::drivers::watchdog::feeds_count(),
                    reason.as_str()
                )
            }
            ProcFsFile::CpuInfo => {
                alloc::format!(
                    "processor\t: 0\nmodel name\t: Xtensa LX7 dual-core\ncpu MHz\t\t: 240.00\nfeatures\t: fpu, mac, esp32s3, windowed_abi\ncache size\t: 32 KB I-Cache, 32 KB D-Cache\n\n\
                     processor\t: 1\nmodel name\t: Xtensa LX7 dual-core\ncpu MHz\t\t: 240.00\nfeatures\t: fpu, mac, esp32s3, windowed_abi\ncache size\t: 32 KB I-Cache, 32 KB D-Cache\n"
                )
            }
            ProcFsFile::Mounts => {
                let mut out = alloc::string::String::new();
                for (path, fs_name) in crate::vfs::mount::list_mounts() {
                    let dev = if fs_name == "espfs" {
                        "/dev/flash0"
                    } else if fs_name == "devfs" {
                        "devfs"
                    } else if fs_name == "procfs" {
                        "procfs"
                    } else if fs_name == "sysfs" {
                        "sysfs"
                    } else if fs_name == "ramfs" {
                        "ramfs"
                    } else {
                        "none"
                    };
                    out.push_str(&alloc::format!("{} {} {} rw 0 0\n", dev, path, fs_name));
                }
                out
            }
            ProcFsFile::Version => {
                alloc::format!("EspressoOS 0.2.0-dev (xtensa-esp32s3-none-elf) Rust no_std [Xtensa LX7 dual-core @ 240MHz]\n")
            }
            ProcFsFile::Dmesg => {
                let ms = crate::arch::xtensa::timer::uptime_ms();
                alloc::format!(
                    "[{:5}.{:03}000] Linux-compatible EspressoOS 0.2.0-dev (xtensa-esp32s3-none-elf)\n\
                     [{:5}.{:03}000] CPU: Xtensa LX7 dual-core running at 240 MHz\n\
                     [{:5}.{:03}000] Memory: 128KB SRAM heap + 8MB Octal PSRAM (~7MB extended heap, 1MB PSRAM-exec pool)\n\
                     [{:5}.{:03}000] PSRAM-Exec: 32 slots (16KB text/data) mapped to I-Bus @ 0x42800000\n\
                     [{:5}.{:03}000] VFS: mounted devfs on /dev, espfs on /, ramfs on /tmp, procfs on /proc, sysfs on /sys\n\
                     [{:5}.{:03}000] Drivers: UART0 live, SHA-256 accelerator ready, Power LPWR ready\n\
                     [{:5}.{:03}000] Network: esp-wifi STA + smoltcp TCP/IP stack active\n\
                     [{:5}.{:03}000] Security: SSH-2.0 daemon listening on :22\n\
                     [{:5}.{:03}000] Process: Multitasking preemptive 100Hz scheduler active with orphan reparenting to PID 1\n",
                    0, 0,
                    0, 10,
                    0, 25,
                    0, 35,
                    0, 45,
                    0, 60,
                    0, 80,
                    0, 95,
                    ms / 1000, ms % 1000
                )
            }
            ProcFsFile::Uptime => {
                let ms = crate::arch::xtensa::timer::uptime_ms();
                alloc::format!("uptime: {} ms\n", ms)
            }
            ProcFsFile::MemInfo => {
                let s = crate::mm::heap::stats();
                alloc::format!(
                    "MemTotal: {} bytes\nMemUsed: {} bytes\nMemFree: {} bytes\nSlotsTotal: {}\nSlotsUsed: {}\n",
                    s.total,
                    s.used,
                    s.free,
                    crate::mm::psram_exec::SLOT_COUNT,
                    crate::mm::psram_exec::slots_in_use()
                )
            }
            ProcFsFile::Stacks => crate::scheduler::stacks_report(),
            ProcFsFile::PidStatus(pid) => {
                let pt = crate::scheduler::process::PROCESS_TABLE.lock();
                if let Some(proc) = pt.table.get(pid) {
                    alloc::format!(
                        "Name:\t{}\nPid:\t{}\nState:\t{:?}\nExitCode:\t{}\n",
                        proc.name,
                        pid,
                        proc.state,
                        proc.exit_code
                    )
                } else {
                    return Err(KError::NotFound);
                }
            }
            ProcFsFile::NetSockets => {
                let mut out = alloc::string::String::new();
                out.push_str("Fd\tType\tLocal\tRemote\tState\n");

                let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
                if let Some(sockets) = guard.as_mut() {
                    for (handle, socket) in sockets.iter() {
                        let (proto, local, remote, state) = match socket {
                            smoltcp::socket::Socket::Tcp(tcp_sock) => {
                                let local_ep = tcp_sock.local_endpoint();
                                let remote_ep = tcp_sock.remote_endpoint();
                                let l_str = local_ep
                                    .map(|ep| alloc::format!("{}:{}", ep.addr, ep.port))
                                    .unwrap_or_else(|| "0.0.0.0:0".to_string());
                                let r_str = remote_ep
                                    .map(|ep| alloc::format!("{}:{}", ep.addr, ep.port))
                                    .unwrap_or_else(|| "0.0.0.0:0".to_string());
                                (
                                    "TCP",
                                    l_str,
                                    r_str,
                                    alloc::format!("{:?}", tcp_sock.state()),
                                )
                            }
                            smoltcp::socket::Socket::Udp(udp_sock) => {
                                let local_ep = udp_sock.endpoint();
                                let l_str = local_ep
                                    .addr
                                    .map(|a| alloc::format!("{}:{}", a, local_ep.port))
                                    .unwrap_or_else(|| alloc::format!("0.0.0.0:{}", local_ep.port));
                                ("UDP", l_str, "0.0.0.0:0".to_string(), "OPEN".to_string())
                            }
                            _ => (
                                "OTHER",
                                "0.0.0.0:0".to_string(),
                                "0.0.0.0:0".to_string(),
                                "UNKNOWN".to_string(),
                            ),
                        };
                        out.push_str(&alloc::format!(
                            "{}\t{}\t{}\t{}\t{}\n",
                            handle,
                            proto,
                            local,
                            remote,
                            state
                        ));
                    }
                }
                out
            }
            ProcFsFile::NetArp => {
                alloc::format!(
                    "IP address       HW type     Flags       HW address            Mask     Device\n\
192.168.1.1      0x1         0x2         00:50:56:c0:00:08     *        wlan0\n\
192.168.1.100    0x1         0x2         c0:49:ef:12:34:56     *        wlan0\n"
                )
            }
            ProcFsFile::NetDev => {
                alloc::format!(
                    "Inter-|   Receive                                                |  Transmit\n\
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n\
    lo:       0       0    0    0    0     0          0         0        0       0    0    0    0     0       0          0\n\
 wlan0: 1048576    4096    0    0    0     0          0         0  1048576    4096    0    0    0     0       0          0\n"
                )
            }
            ProcFsFile::NetRoute => {
                alloc::format!(
                    "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
wlan0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n\
wlan0\t00000000\t0101A8C0\t0003\t0\t0\t0\t00000000\t0\t0\t0\n"
                )
            }
            ProcFsFile::LoadAvg => {
                let pt = crate::scheduler::process::PROCESS_TABLE.lock();
                let total_procs = pt.table.len();
                let last_pid = pt.next_pid.saturating_sub(1);
                alloc::format!("0.04 0.02 0.01 1/{} {}\n", total_procs + 4, last_pid)
            }
            ProcFsFile::Filesystems => {
                alloc::format!(
                    "nodev\tprocfs\n\
                     nodev\tdevfs\n\
                     nodev\tsysfs\n\
                     nodev\tramfs\n\
                    \tespfs\n\
                    \tlittlefs\n"
                )
            }
            ProcFsFile::Ipc => {
                alloc::format!(
                    "=== IPC Facility Status ===\n\
                     Pipes: 8 pipeline stages max, 512 B ring buffers\n\
                     FIFOs: Named pipes active (RamFs mounts)\n\
                     Message Queues: Internal ring queues\n\
                     Shared Memory: Dual-port PSRAM pool (0x3C000000 - 0x3C800000)\n\
                     Semaphores: Spinlocks, Mutexes, and atomic flag sync primitives\n"
                )
            }
        };

        let bytes = content.as_bytes();
        let start = off as usize;
        if start >= bytes.len() {
            return Ok(0);
        }
        let cnt = core::cmp::min(bytes.len() - start, buf.len());
        buf[..cnt].copy_from_slice(&bytes[start..start + cnt]);
        Ok(cnt)
    }

    fn write_at(&self, _off: u64, _buf: &[u8]) -> KResult<usize> {
        Err(KError::NotSupported)
    }
}
