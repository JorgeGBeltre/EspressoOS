#![allow(dead_code)]

pub use alloc::boxed::Box;
pub use alloc::string::String;
pub use alloc::sync::Arc;
pub use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KError {

    NoMem,

    NotFound,

    AlreadyExists,

    NotADirectory,

    IsADirectory,

    InvalidArgument,

    PermissionDenied,

    NotSupported,

    WouldBlock,

    Busy,

    IoError,

    BadFd,

    NameTooLong,

    NoSpace,

    Corrupt,

    Timeout,

    Fault,

    TableFull,
}

impl KError {

    pub const fn as_errno(self) -> isize {
        match self {
            KError::NotFound => -2,
            KError::IoError => -5,
            KError::BadFd => -9,
            KError::NoMem => -12,
            KError::PermissionDenied => -13,
            KError::Fault => -14,
            KError::Busy => -16,
            KError::AlreadyExists => -17,
            KError::NotADirectory => -20,
            KError::IsADirectory => -21,
            KError::InvalidArgument => -22,
            KError::TableFull => -23,
            KError::NoSpace => -28,
            KError::NameTooLong => -36,
            KError::WouldBlock => -11,
            KError::Timeout => -110,
            KError::NotSupported => -95,
            KError::Corrupt => -84,
        }
    }
}

pub type KResult<T> = Result<T, KError>;

pub mod layout {

    pub const FLASH_SIZE: u32 = 0x0100_0000;
    pub const PART_TABLE_OFFSET: u32 = 0x0000_8000;
    pub const NVS_OFFSET: u32 = 0x0000_9000;
    pub const NVS_SIZE: u32 = 0x0000_6000;
    pub const OTADATA_OFFSET: u32 = 0x0000_F000;
    pub const OTADATA_SIZE: u32 = 0x0000_2000;
    pub const FACTORY_OFFSET: u32 = 0x0002_0000;
    pub const FACTORY_SIZE: u32 = 0x0040_0000;
    pub const OTA0_OFFSET: u32 = 0x0042_0000;
    pub const OTA0_SIZE: u32 = 0x0040_0000;
    pub const FS_OFFSET: u32 = 0x0082_0000;
    pub const FS_SIZE: u32 = 0x007D_0000;
    pub const COREDUMP_OFFSET: u32 = 0x00FF_0000;
    pub const COREDUMP_SIZE: u32 = 0x0001_0000;

    pub const FLASH_SECTOR_SIZE: usize = 4096;

    pub const KERNEL_HEAP_SIZE: usize = 128 * 1024;
    pub const PSRAM_SIZE: usize = 8 * 1024 * 1024;
    pub const DEFAULT_STACK_SIZE: usize = 8 * 1024;
}
