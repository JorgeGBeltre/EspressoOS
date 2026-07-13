#![allow(dead_code)]

pub const PARTITION_TABLE_OFFSET: u32 = 0x8000;
pub const PARTITION_MAGIC: u16 = 0xAA50;

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum PartitionType {
    App = 0,
    Data = 1,
}

#[repr(C, packed)]
pub struct PartitionEntry {
    pub magic: u16,
    pub ptype: u8,
    pub subtype: u8,
    pub offset: u32,
    pub size: u32,
    pub label: [u8; 16],
    pub flags: u32,
}
