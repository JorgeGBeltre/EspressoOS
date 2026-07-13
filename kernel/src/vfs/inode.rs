#![allow(dead_code)]

use crate::prelude::*;

pub type VfsError = KError;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InodeKind {

    File,

    Dir,

    Device,

    Symlink,
}

#[derive(Clone, Debug)]
pub struct DirEntry {

    pub name: String,

    pub kind: InodeKind,

    pub ino: u64,
}

pub trait Inode: Send + Sync {

    fn kind(&self) -> InodeKind;

    fn size(&self) -> u64;

    fn as_socket(&self) -> Option<smoltcp::iface::SocketHandle> {
        None
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize>;

    fn write_at(&self, off: u64, buf: &[u8]) -> KResult<usize>;

    fn truncate(&self, _len: u64) -> KResult<()> {
        Err(KError::NotSupported)
    }

    fn readdir(&self, _index: usize) -> KResult<Option<DirEntry>> {
        Err(KError::NotADirectory)
    }

    fn lookup(&self, _name: &str) -> KResult<Arc<dyn Inode>> {
        Err(KError::NotADirectory)
    }

    fn create(&self, _name: &str, _kind: InodeKind) -> KResult<Arc<dyn Inode>> {
        Err(KError::NotADirectory)
    }

    fn unlink(&self, _name: &str) -> KResult<()> {
        Err(KError::NotADirectory)
    }

    fn sync(&self) -> KResult<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FsStat {

    pub total_bytes: u64,

    pub used_bytes: u64,

    pub block_size: u32,
}

pub trait FileSystem: Send + Sync {

    fn name(&self) -> &str;

    fn root(&self) -> Arc<dyn Inode>;

    fn sync(&self) -> KResult<()>;

    fn stat(&self) -> FsStat;
}
