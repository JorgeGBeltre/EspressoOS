#![allow(dead_code)]

use crate::drivers::flash;
use crate::prelude::*;
use crate::vfs::inode::{DirEntry, FileSystem, FsStat, Inode, InodeKind};

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

    fn abs_offset(&self, block: u32, off: u32, size: u32) -> KResult<u32> {
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

    fn dev_read(&self, block: u32, off: u32, buf: &mut [u8]) -> KResult<()> {
        let abs = self.abs_offset(block, off, buf.len() as u32)?;
        flash::read(abs, buf)
    }

    fn dev_prog(&self, block: u32, off: u32, buf: &[u8]) -> KResult<()> {
        let abs = self.abs_offset(block, off, buf.len() as u32)?;
        flash::write(abs, buf)
    }

    fn dev_erase(&self, block: u32) -> KResult<()> {
        let abs = self.abs_offset(block, 0, self.block_size)?;
        flash::erase_sector(abs)
    }

    fn dev_sync(&self) -> KResult<()> {
        Ok(())
    }
}

struct LfsRoot;

impl Inode for LfsRoot {
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
    fn readdir(&self, _index: usize) -> KResult<Option<DirEntry>> {

        Ok(None)
    }
    fn lookup(&self, _name: &str) -> KResult<Arc<dyn Inode>> {
        Err(KError::NotFound)
    }
    fn create(&self, _name: &str, _kind: InodeKind) -> KResult<Arc<dyn Inode>> {
        Err(KError::NotSupported)
    }
    fn unlink(&self, _name: &str) -> KResult<()> {
        Err(KError::NotSupported)
    }
}

pub struct LittleFs {
    config: LfsConfig,
    root: Arc<LfsRoot>,
}

impl LittleFs {

    pub fn mount(offset: u32, len: u32, format_if_empty: bool) -> KResult<Arc<LittleFs>> {
        Self::validar_region(offset, len)?;
        let config = LfsConfig::for_region(offset, len);

        let _ = format_if_empty;

        Ok(Arc::new(LittleFs {
            config,
            root: Arc::new(LfsRoot),
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
        let r: Arc<dyn Inode> = self.root.clone();
        r
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
