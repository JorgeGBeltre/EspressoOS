#![allow(dead_code)]

use crate::prelude::*;
use crate::arch::xtensa::sync::Mutex;
use super::inode::{DirEntry, FileSystem, FsStat, Inode, InodeKind};
use super::mount::MAX_NAME_LEN;

pub trait Device: Send + Sync {

    fn read(&self, off: u64, buf: &mut [u8]) -> KResult<usize>;

    fn write(&self, off: u64, buf: &[u8]) -> KResult<usize>;

    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<usize> {
        let _ = (cmd, arg);
        Err(KError::NotSupported)
    }
}

type DevTable = Arc<Mutex<Vec<(String, Arc<dyn Device>)>>>;

pub struct DevFs {

    table: DevTable,

    root: Arc<DevRoot>,
}

impl DevFs {

    pub fn new() -> Arc<DevFs> {
        let table: DevTable = Arc::new(Mutex::new(Vec::new()));
        let root = Arc::new(DevRoot {
            table: table.clone(),
        });
        Arc::new(DevFs { table, root })
    }
}

impl FileSystem for DevFs {
    fn name(&self) -> &str {
        "devfs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sync(&self) -> KResult<()> {
        Ok(())
    }

    fn stat(&self) -> FsStat {

        FsStat::default()
    }
}

struct DevRoot {
    table: DevTable,
}

impl Inode for DevRoot {
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
        Err(KError::IsADirectory)
    }

    fn readdir(&self, index: usize) -> KResult<Option<DirEntry>> {
        let table = self.table.lock();
        match table.get(index) {
            Some((name, _)) => Ok(Some(DirEntry {
                name: name.clone(),
                kind: InodeKind::Device,
                ino: (index as u64).saturating_add(1),
            })),
            None => Ok(None),
        }
    }

    fn lookup(&self, name: &str) -> KResult<Arc<dyn Inode>> {
        let table = self.table.lock();
        for (n, dev) in table.iter() {
            if n == name {
                return Ok(Arc::new(DevNode { dev: dev.clone() }));
            }
        }
        Err(KError::NotFound)
    }

    fn create(&self, _name: &str, _kind: InodeKind) -> KResult<Arc<dyn Inode>> {

        Err(KError::PermissionDenied)
    }

    fn unlink(&self, _name: &str) -> KResult<()> {
        Err(KError::PermissionDenied)
    }
}

struct DevNode {
    dev: Arc<dyn Device>,
}

impl Inode for DevNode {
    fn kind(&self) -> InodeKind {
        InodeKind::Device
    }

    fn size(&self) -> u64 {
        0
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        self.dev.read(off, buf)
    }

    fn write_at(&self, off: u64, buf: &[u8]) -> KResult<usize> {
        self.dev.write(off, buf)
    }

    fn truncate(&self, _len: u64) -> KResult<()> {

        Ok(())
    }

}

pub fn register(devfs: &Arc<DevFs>, name: &str, dev: Arc<dyn Device>) -> KResult<()> {
    if name.len() > MAX_NAME_LEN {
        return Err(KError::NameTooLong);
    }
    let mut table = devfs.table.lock();
    if table.iter().any(|(n, _)| n == name) {
        return Err(KError::AlreadyExists);
    }
    table.push((String::from(name), dev));
    Ok(())
}

pub fn init() -> KResult<Arc<DevFs>> {
    let devfs = DevFs::new();
    register(&devfs, "null", Arc::new(NullDevice))?;
    register(&devfs, "zero", Arc::new(ZeroDevice))?;
    register(&devfs, "console", Arc::new(ConsoleDevice))?;
    // Buses (Fase 3): los nodos existen aunque el bus no esté inicializado;
    // las operaciones devuelven IoError hasta que `main` llame a `init`.
    register(&devfs, "i2c0", crate::drivers::i2c::devfs_device())?;
    register(&devfs, "spi0", crate::drivers::spi::devfs_device())?;
    Ok(devfs)
}

struct NullDevice;
impl Device for NullDevice {
    fn read(&self, _off: u64, _buf: &mut [u8]) -> KResult<usize> {
        Ok(0)
    }
    fn write(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        Ok(buf.len())
    }
}

struct ZeroDevice;
impl Device for ZeroDevice {
    fn read(&self, _off: u64, buf: &mut [u8]) -> KResult<usize> {
        for b in buf.iter_mut() {
            *b = 0;
        }
        Ok(buf.len())
    }
    fn write(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        Ok(buf.len())
    }
}

struct ConsoleDevice;
impl Device for ConsoleDevice {
    fn read(&self, _off: u64, buf: &mut [u8]) -> KResult<usize> {
        Ok(crate::drivers::uart::read(buf))
    }
    fn write(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        Ok(crate::drivers::uart::write(buf))
    }
}
