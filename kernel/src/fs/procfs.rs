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

        let pids: Vec<u32> = {
            let pt = crate::scheduler::process::PROCESS_TABLE.lock();
            pt.table.keys().copied().collect()
        };

        if index < 7 {
            return Ok(None);
        }
        let pid_idx = index - 7;
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
        Ok(None)
    }
}

enum ProcFsFile {
    Uptime,
    MemInfo,
    Stacks,
    PidStatus(u32),
    NetSockets,
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
