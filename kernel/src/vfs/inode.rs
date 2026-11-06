#![allow(dead_code)]

use crate::prelude::*;

pub type VfsError = KError;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InodeKind {
    File,

    Dir,

    Device,

    Symlink,

    Fifo,
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,

    pub kind: InodeKind,

    pub ino: u64,
}

pub const fn kind_code(kind: InodeKind) -> u8 {
    match kind {
        InodeKind::File => 1,
        InodeKind::Dir => 2,
        InodeKind::Device => 3,
        InodeKind::Symlink => 4,
        InodeKind::Fifo => 5,
    }
}

pub const fn default_mode(kind: InodeKind) -> u16 {
    match kind {
        InodeKind::Dir => 0o0755,
        InodeKind::Symlink => 0o0777,
        InodeKind::Device => 0o0666,
        InodeKind::Fifo => 0o0660,
        InodeKind::File => 0o0644,
    }
}

/// Fixed 32-byte metadata layout shared across kernel VFS and userland `stat`/`lstat`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StatMeta {
    pub ino: u64,
    pub size: u64,
    pub mode: u16,
    pub kind: u8,
    pub nlink: u8,
    pub uid: u16,
    pub gid: u16,
    pub mtime: u64,
}

pub trait Inode: Send + Sync {
    fn kind(&self) -> InodeKind;

    fn size(&self) -> u64;

    fn inode_key(&self) -> (usize, u64) {
        (self as *const Self as *const () as usize, 0)
    }

    fn meta(&self) -> StatMeta {
        let k = self.kind();
        StatMeta {
            ino: 1,
            size: self.size(),
            mode: default_mode(k),
            kind: kind_code(k),
            nlink: if k == InodeKind::Dir { 2 } else { 1 },
            uid: 0,
            gid: 0,
            mtime: 0,
        }
    }

    fn readlink(&self) -> KResult<String> {
        Err(KError::InvalidArgument)
    }

    fn create_symlink(&self, _name: &str, _target: &str) -> KResult<Arc<dyn Inode>> {
        Err(KError::NotSupported)
    }

    fn set_mode(&self, _mode: u16) -> KResult<()> {
        Err(KError::NotSupported)
    }

    fn set_owner(&self, _uid: Option<u16>, _gid: Option<u16>) -> KResult<()> {
        Err(KError::NotSupported)
    }

    fn as_socket(&self) -> Option<smoltcp::iface::SocketHandle> {
        None
    }

    fn is_udp_socket(&self) -> bool {
        false
    }

    fn is_icmp_socket(&self) -> bool {
        false
    }

    fn set_socket_remote_endpoint(&self, _endpoint: smoltcp::wire::IpEndpoint) -> KResult<()> {
        Err(KError::NotSupported)
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize>;

    fn write_at(&self, off: u64, buf: &[u8]) -> KResult<usize>;

    fn truncate(&self, _len: u64) -> KResult<()> {
        Err(KError::NotSupported)
    }

    fn ioctl(&self, _cmd: u32, _arg: usize) -> KResult<usize> {
        Err(KError::NotSupported)
    }

    fn bind(&self, _port: u16) -> KResult<()> {
        Err(KError::NotSupported)
    }

    fn listen(&self, _backlog: i32) -> KResult<()> {
        Err(KError::NotSupported)
    }

    fn accept(&self) -> KResult<Arc<dyn Inode>> {
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

    fn rename(&self, _old_rel: &str, _new_rel: &str) -> KResult<()> {
        Err(KError::NotSupported)
    }
}
