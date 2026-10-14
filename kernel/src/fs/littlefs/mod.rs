#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::drivers::flash;
use crate::prelude::*;
use crate::vfs::inode::{DirEntry, FileSystem, FsStat, Inode, InodeKind};
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU32, Ordering};

static NEXT_INO: AtomicU32 = AtomicU32::new(2);

fn alloc_ino() -> u64 {
    NEXT_INO.fetch_add(1, Ordering::Relaxed) as u64
}

#[derive(Clone, Copy, Debug)]
pub struct LfsConfig {
    pub region_offset: u32,
    pub region_len: u32,
    pub read_size: u32,
    pub prog_size: u32,
    pub block_size: u32,
    pub block_count: u32,
    pub block_cycles: i32,
    pub cache_size: u32,
    pub lookahead_size: u32,
}

impl LfsConfig {
    pub const fn for_region(offset: u32, len: u32) -> LfsConfig {
        let block_size = flash::SECTOR_SIZE as u32;
        LfsConfig {
            region_offset: offset,
            region_len: len,
            read_size: 16,
            prog_size: 16,
            block_size,
            block_count: len / block_size,
            block_cycles: 500,
            cache_size: 256,
            lookahead_size: 32,
        }
    }

    pub fn abs_offset(&self, block: u32, off: u32, size: u32) -> KResult<u32> {
        if block >= self.block_count {
            return Err(KError::InvalidArgument);
        }

        match off.checked_add(size) {
            Some(fin) if fin <= self.block_size => {}
            _ => return Err(KError::InvalidArgument),
        }
        let block_byte = block
            .checked_mul(self.block_size)
            .ok_or(KError::InvalidArgument)?;
        let within = block_byte.checked_add(off).ok_or(KError::InvalidArgument)?;
        let abs = self
            .region_offset
            .checked_add(within)
            .ok_or(KError::InvalidArgument)?;
        Ok(abs)
    }

    pub fn dev_read(&self, block: u32, off: u32, buf: &mut [u8]) -> KResult<()> {
        let abs = self.abs_offset(block, off, buf.len() as u32)?;
        flash::read(abs, buf)
    }

    pub fn dev_prog(&self, block: u32, off: u32, buf: &[u8]) -> KResult<()> {
        let abs = self.abs_offset(block, off, buf.len() as u32)?;
        flash::write(abs, buf)
    }

    pub fn dev_erase(&self, block: u32) -> KResult<()> {
        let abs = self.abs_offset(block, 0, self.block_size)?;
        flash::erase_sector(abs)
    }

    pub fn dev_sync(&self) -> KResult<()> {
        Ok(())
    }
}

pub struct LfsFile {
    ino: u64,
    data: Mutex<Vec<u8>>,
}

impl LfsFile {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            ino: alloc_ino(),
            data: Mutex::new(Vec::new()),
        })
    }
}

impl Inode for LfsFile {
    fn kind(&self) -> InodeKind {
        InodeKind::File
    }

    fn size(&self) -> u64 {
        self.data.lock().len() as u64
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let data = self.data.lock();
        let off = off as usize;
        if off >= data.len() {
            return Ok(0);
        }
        let avail = &data[off..];
        let n = buf.len().min(avail.len());
        buf[..n].copy_from_slice(&avail[..n]);
        Ok(n)
    }

    fn write_at(&self, off: u64, buf: &[u8]) -> KResult<usize> {
        let mut data = self.data.lock();
        let off = off as usize;
        let end = off.checked_add(buf.len()).ok_or(KError::NoMem)?;
        if end > data.len() {
            data.resize(end, 0);
        }
        data[off..end].copy_from_slice(buf);
        Ok(buf.len())
    }

    fn truncate(&self, len: u64) -> KResult<()> {
        let mut data = self.data.lock();
        data.resize(len as usize, 0);
        Ok(())
    }

    fn sync(&self) -> KResult<()> {
        Ok(())
    }
}

pub struct LfsDir {
    ino: u64,
    entries: Mutex<BTreeMap<String, Arc<dyn Inode>>>,
}

impl LfsDir {
    pub fn new(ino: u64) -> Arc<Self> {
        Arc::new(Self {
            ino,
            entries: Mutex::new(BTreeMap::new()),
        })
    }
}

impl Inode for LfsDir {
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

    fn truncate(&self, _len: u64) -> KResult<()> {
        Err(KError::IsADirectory)
    }

    fn readdir(&self, index: usize) -> KResult<Option<DirEntry>> {
        if index == 0 {
            return Ok(Some(DirEntry {
                name: String::from("."),
                kind: InodeKind::Dir,
                ino: self.ino,
            }));
        }
        if index == 1 {
            return Ok(Some(DirEntry {
                name: String::from(".."),
                kind: InodeKind::Dir,
                ino: 1,
            }));
        }
        let entries = self.entries.lock();
        let child_idx = index - 2;
        if let Some((name, node)) = entries.iter().nth(child_idx) {
            Ok(Some(DirEntry {
                name: name.clone(),
                kind: node.kind(),
                ino: alloc_ino(),
            }))
        } else {
            Ok(None)
        }
    }

    fn lookup(&self, name: &str) -> KResult<Arc<dyn Inode>> {
        let entries = self.entries.lock();
        match entries.get(name) {
            Some(node) => Ok(node.clone()),
            None => Err(KError::NotFound),
        }
    }

    fn create(&self, name: &str, kind: InodeKind) -> KResult<Arc<dyn Inode>> {
        let mut entries = self.entries.lock();
        if entries.contains_key(name) {
            return Err(KError::AlreadyExists);
        }
        let node: Arc<dyn Inode> = match kind {
            InodeKind::File => LfsFile::new(),
            InodeKind::Dir => LfsDir::new(alloc_ino()),
            _ => return Err(KError::NotSupported),
        };
        entries.insert(String::from(name), node.clone());
        Ok(node)
    }

    fn unlink(&self, name: &str) -> KResult<()> {
        let mut entries = self.entries.lock();
        if entries.remove(name).is_some() {
            Ok(())
        } else {
            Err(KError::NotFound)
        }
    }

    fn sync(&self) -> KResult<()> {
        Ok(())
    }
}

pub struct LittleFs {
    config: LfsConfig,
    root: Arc<LfsDir>,
}

impl LittleFs {
    pub fn mount(offset: u32, len: u32, _format_if_empty: bool) -> KResult<Arc<LittleFs>> {
        Self::validar_region(offset, len)?;
        let config = LfsConfig::for_region(offset, len);

        Ok(Arc::new(LittleFs {
            config,
            root: LfsDir::new(1),
        }))
    }

    pub fn format(offset: u32, len: u32) -> KResult<()> {
        Self::validar_region(offset, len)?;
        let sector = flash::SECTOR_SIZE as u32;
        let fin = offset.checked_add(len).ok_or(KError::InvalidArgument)?;
        let mut pos = offset;
        while pos < fin {
            flash::erase_sector(pos)?;
            pos = pos.checked_add(sector).ok_or(KError::InvalidArgument)?;
        }
        Ok(())
    }

    fn validar_region(offset: u32, len: u32) -> KResult<()> {
        let sector = flash::SECTOR_SIZE as u32;
        if len == 0 || sector == 0 {
            return Err(KError::InvalidArgument);
        }
        if offset % sector != 0 || len % sector != 0 {
            return Err(KError::InvalidArgument);
        }
        let fin = offset.checked_add(len).ok_or(KError::InvalidArgument)?;
        if fin > layout::FLASH_SIZE {
            return Err(KError::InvalidArgument);
        }
        Ok(())
    }
}

impl FileSystem for LittleFs {
    fn name(&self) -> &str {
        "littlefs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sync(&self) -> KResult<()> {
        Ok(())
    }

    fn stat(&self) -> FsStat {
        FsStat {
            total_bytes: self.config.region_len as u64,
            used_bytes: 0,
            block_size: self.config.block_size,
        }
    }
}
