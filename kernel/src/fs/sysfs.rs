#![allow(dead_code)]

use crate::prelude::*;
use crate::vfs::inode::{DirEntry, FileSystem, FsStat, Inode, InodeKind};
use alloc::string::ToString;
use alloc::vec;

pub struct SysFs {
    root: Arc<SysFsRoot>,
}

impl SysFs {
    pub fn new() -> Self {
        Self {
            root: Arc::new(SysFsRoot),
        }
    }
}

impl FileSystem for SysFs {
    fn name(&self) -> &str {
        "sysfs"
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

struct SysFsRoot;

impl Inode for SysFsRoot {
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
        if name == "kernel" {
            return Ok(Arc::new(SysFsFile::KernelInfo));
        }
        if name == "smp" {
            return Ok(Arc::new(SysFsFile::Smp));
        }
        if name == "pms" {
            return Ok(Arc::new(SysFsFile::Pms));
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
                name: "kernel".to_string(),
                kind: InodeKind::File,
                ino: 2,
            }));
        }
        if index == 3 {
            return Ok(Some(DirEntry {
                name: "smp".to_string(),
                kind: InodeKind::File,
                ino: 3,
            }));
        }
        if index == 4 {
            return Ok(Some(DirEntry {
                name: "pms".to_string(),
                kind: InodeKind::File,
                ino: 4,
            }));
        }
        Ok(None)
    }
}

enum SysFsFile {
    KernelInfo,
    Smp,
    Pms,
}

impl Inode for SysFsFile {
    fn kind(&self) -> InodeKind {
        InodeKind::File
    }

    fn size(&self) -> u64 {
        0
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        // Estado por lectura (D-8). Las ACCIONES feature-gated (`pms world1`) siguen en el
        // shell del kernel hasta SP4; aquí solo se expone el estado.
        let content: String = match self {
            SysFsFile::KernelInfo => String::from("EspressoOS Kernel v0.1.0\n"),
            SysFsFile::Smp => {
                let core = crate::scheduler::core_sync::current_core_id();
                if cfg!(feature = "smp") {
                    let running = crate::scheduler::core_sync::is_running();
                    alloc::format!(
                        "smp: enabled\ncore: {}\napp_cpu: {}\n",
                        core,
                        if running { "active" } else { "not started" }
                    )
                } else {
                    alloc::format!("smp: disabled (build --features smp)\ncore: {}\n", core)
                }
            }
            SysFsFile::Pms => {
                if cfg!(feature = "pms") {
                    match crate::mm::mpu::report() {
                        Some(s) => alloc::format!("pms: enabled\n{}\n", s),
                        None => String::from("pms: enabled but unavailable\n"),
                    }
                } else {
                    String::from("pms: disabled (build --features pms)\n")
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
