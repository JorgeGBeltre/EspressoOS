#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;
use crate::vfs::inode::{DirEntry, FileSystem, FsStat, Inode, InodeKind};
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU32, Ordering};

static NEXT_INO: AtomicU32 = AtomicU32::new(1);

fn alloc_ino() -> u64 {
    NEXT_INO.fetch_add(1, Ordering::Relaxed) as u64
}

enum RamBody {
    File(Vec<u8>),

    Dir(BTreeMap<String, Arc<RamNode>>),
}

struct RamNode {
    ino: u64,

    kind: InodeKind,

    body: Mutex<RamBody>,
}

impl RamNode {
    fn new_dir() -> Arc<RamNode> {
        Arc::new(RamNode {
            ino: alloc_ino(),
            kind: InodeKind::Dir,
            body: Mutex::new(RamBody::Dir(BTreeMap::new())),
        })
    }

    fn new_file() -> Arc<RamNode> {
        Arc::new(RamNode {
            ino: alloc_ino(),
            kind: InodeKind::File,
            body: Mutex::new(RamBody::File(Vec::new())),
        })
    }
}

impl Inode for RamNode {
    fn kind(&self) -> InodeKind {
        self.kind
    }

    fn size(&self) -> u64 {
        let body = self.body.lock();
        match &*body {
            RamBody::File(data) => data.len() as u64,

            RamBody::Dir(_) => 0,
        }
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let body = self.body.lock();
        match &*body {
            RamBody::File(data) => {
                let start = match usize::try_from(off) {
                    Ok(v) => v,
                    Err(_) => return Ok(0),
                };
                if start >= data.len() {
                    return Ok(0);
                }
                let disponible = data.len() - start;
                let n = core::cmp::min(disponible, buf.len());

                buf[..n].copy_from_slice(&data[start..start + n]);
                Ok(n)
            }
            RamBody::Dir(_) => Err(KError::IsADirectory),
        }
    }

    fn write_at(&self, off: u64, buf: &[u8]) -> KResult<usize> {
        let mut body = self.body.lock();
        match &mut *body {
            RamBody::File(data) => {
                let start = usize::try_from(off).map_err(|_| KError::InvalidArgument)?;

                let end = start.checked_add(buf.len()).ok_or(KError::NoMem)?;
                if end > data.len() {
                    let extra = end - data.len();
                    data.try_reserve(extra).map_err(|_| KError::NoMem)?;
                    data.resize(end, 0);
                }

                data[start..end].copy_from_slice(buf);
                Ok(buf.len())
            }
            RamBody::Dir(_) => Err(KError::IsADirectory),
        }
    }

    fn truncate(&self, len: u64) -> KResult<()> {
        let mut body = self.body.lock();
        match &mut *body {
            RamBody::File(data) => {
                let n = usize::try_from(len).map_err(|_| KError::InvalidArgument)?;
                if n > data.len() {
                    let extra = n - data.len();
                    data.try_reserve(extra).map_err(|_| KError::NoMem)?;
                }
                data.resize(n, 0);
                Ok(())
            }
            RamBody::Dir(_) => Err(KError::IsADirectory),
        }
    }

    fn readdir(&self, index: usize) -> KResult<Option<DirEntry>> {
        let body = self.body.lock();
        match &*body {
            RamBody::Dir(children) => match children.iter().nth(index) {
                Some((name, node)) => Ok(Some(DirEntry {
                    name: name.clone(),
                    kind: node.kind,
                    ino: node.ino,
                })),
                None => Ok(None),
            },
            RamBody::File(_) => Err(KError::NotADirectory),
        }
    }

    fn lookup(&self, name: &str) -> KResult<Arc<dyn Inode>> {
        let body = self.body.lock();
        match &*body {
            RamBody::Dir(children) => match children.get(name) {
                Some(node) => {
                    let out: Arc<dyn Inode> = node.clone();
                    Ok(out)
                }
                None => Err(KError::NotFound),
            },
            RamBody::File(_) => Err(KError::NotADirectory),
        }
    }

    fn create(&self, name: &str, kind: InodeKind) -> KResult<Arc<dyn Inode>> {
        let mut body = self.body.lock();
        match &mut *body {
            RamBody::Dir(children) => {
                if children.contains_key(name) {
                    return Err(KError::AlreadyExists);
                }
                let node = match kind {
                    InodeKind::File => RamNode::new_file(),
                    InodeKind::Dir => RamNode::new_dir(),

                    InodeKind::Device | InodeKind::Symlink => return Err(KError::NotSupported),
                };
                children.insert(String::from(name), node.clone());
                let out: Arc<dyn Inode> = node;
                Ok(out)
            }
            RamBody::File(_) => Err(KError::NotADirectory),
        }
    }

    fn unlink(&self, name: &str) -> KResult<()> {
        let mut body = self.body.lock();
        match &mut *body {
            RamBody::Dir(children) => {
                let vacio = match children.get(name) {
                    None => return Err(KError::NotFound),
                    Some(node) => {
                        let child_body = node.body.lock();
                        match &*child_body {
                            RamBody::Dir(sub) => sub.is_empty(),
                            RamBody::File(_) => true,
                        }
                    }
                };
                if !vacio {
                    return Err(KError::Busy);
                }

                children.remove(name);
                Ok(())
            }
            RamBody::File(_) => Err(KError::NotADirectory),
        }
    }
}

pub struct RamFs {
    root: Arc<RamNode>,
}

impl RamFs {
    pub fn new() -> Arc<RamFs> {
        Arc::new(RamFs {
            root: RamNode::new_dir(),
        })
    }

    fn used_bytes(node: &Arc<RamNode>) -> u64 {
        let body = node.body.lock();
        match &*body {
            RamBody::File(data) => data.len() as u64,
            RamBody::Dir(children) => {
                let mut total: u64 = 0;
                for (_, child) in children.iter() {
                    total = total.saturating_add(Self::used_bytes(child));
                }
                total
            }
        }
    }
}

impl FileSystem for RamFs {
    fn name(&self) -> &str {
        "ramfs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        let r: Arc<dyn Inode> = self.root.clone();
        r
    }

    fn sync(&self) -> KResult<()> {
        Ok(())
    }

    fn stat(&self) -> FsStat {
        let usado = RamFs::used_bytes(&self.root);
        FsStat {
            total_bytes: usado,
            used_bytes: usado,
            block_size: 1,
        }
    }
}
