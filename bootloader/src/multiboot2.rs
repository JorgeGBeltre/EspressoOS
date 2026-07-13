#![allow(dead_code)]

pub const ESP_IMAGE_MAGIC: u8 = 0xE9;

#[repr(C, packed)]
pub struct EspImageHeader {
    pub magic: u8,
    pub segment_count: u8,
    pub spi_mode: u8,
    pub spi_speed_size: u8,
    pub entry_addr: u32,
}

#[repr(C, packed)]
pub struct EspSegmentHeader {
    pub load_addr: u32,
    pub length: u32,
}
