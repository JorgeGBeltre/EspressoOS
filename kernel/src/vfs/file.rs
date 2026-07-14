#![allow(dead_code)]

use crate::prelude::*;
use super::inode::Inode;

pub type Fd = i32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OpenFlags(pub u32);

impl OpenFlags {

    pub const RDONLY: OpenFlags = OpenFlags(0x0001);

    pub const WRONLY: OpenFlags = OpenFlags(0x0002);

    pub const RDWR: OpenFlags = OpenFlags(0x0003);

    pub const CREATE: OpenFlags = OpenFlags(0x0100);

    pub const APPEND: OpenFlags = OpenFlags(0x0200);

    pub const TRUNC: OpenFlags = OpenFlags(0x0400);

    pub const fn contains(self, f: OpenFlags) -> bool {
        (self.0 & f.0) == f.0
    }
}

pub enum SeekFrom {

    Start(u64),

    Current(i64),

    End(i64),
}

#[derive(Clone)]
pub struct OpenFile {

    pub inode: Arc<dyn Inode>,

    pub offset: u64,

    pub readable: bool,

    pub writable: bool,

    pub append: bool,
}

fn offset_add(base: u64, delta: i64) -> KResult<u64> {
    if delta >= 0 {
        base.checked_add(delta as u64).ok_or(KError::InvalidArgument)
    } else {

        base.checked_sub(delta.unsigned_abs()).ok_or(KError::InvalidArgument)
    }
}

impl OpenFile {

    pub fn new(inode: Arc<dyn Inode>, flags: OpenFlags) -> KResult<Self> {
        let readable = flags.contains(OpenFlags::RDONLY);
        let writable = flags.contains(OpenFlags::WRONLY);
        if !readable && !writable {

            return Err(KError::InvalidArgument);
        }
        let append = flags.contains(OpenFlags::APPEND);

        if flags.contains(OpenFlags::TRUNC) && writable {
            inode.truncate(0)?;
        }

        Ok(Self {
            inode,
            offset: 0,
            readable,
            writable,
            append,
        })
    }

    pub fn read(&mut self, buf: &mut [u8]) -> KResult<usize> {
        if !self.readable {
            return Err(KError::PermissionDenied);
        }
        let n = self.inode.read_at(self.offset, buf)?;
        self.offset = self
            .offset
            .checked_add(n as u64)
            .ok_or(KError::InvalidArgument)?;
        Ok(n)
    }

    pub fn write(&mut self, buf: &[u8]) -> KResult<usize> {
        if !self.writable {
            return Err(KError::PermissionDenied);
        }
        if self.append {
            self.offset = self.inode.size();
        }
        let n = self.inode.write_at(self.offset, buf)?;
        self.offset = self
            .offset
            .checked_add(n as u64)
            .ok_or(KError::InvalidArgument)?;
        Ok(n)
    }

    pub fn seek(&mut self, pos: SeekFrom) -> KResult<u64> {
        let new = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(d) => offset_add(self.offset, d)?,
            SeekFrom::End(d) => offset_add(self.inode.size(), d)?,
        };
        self.offset = new;
        Ok(new)
    }
}
