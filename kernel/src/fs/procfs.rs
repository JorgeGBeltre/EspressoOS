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
        
        let pids: Vec<u32> = {
            let pt = crate::scheduler::process::PROCESS_TABLE.lock();
            pt.table.keys().copied().collect()
        };
        
        let pid_idx = index - 4;
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

enum ProcFsFile {
    Uptime,
    MemInfo,
    PidStatus(u32),
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
                let size = crate::mm::heap::size();
                alloc::format!("MemTotal: {} bytes\nMemFree: {} bytes\n", size, size)
            }
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
