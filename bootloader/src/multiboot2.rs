//! Formato de imagen de arranque (ESQUELETO).
//!
//! NOTA IMPORTANTE: el nombre "multiboot2" se conserva por continuidad con el
//! plan, pero el ESP32-S3 NO usa Multiboot2 (eso es de GRUB/PC). La ROM del
//! chip espera el *formato de imagen de Espressif*. Aquí se modelan sus
//! estructuras, que es lo que realmente hay que leer/emitir.
//!
//! Referencia: cabecera de imagen de esp-idf (`esp_image_header_t`).
#![allow(dead_code)]

/// Magic byte al inicio de toda imagen de aplicación del ESP32.
pub const ESP_IMAGE_MAGIC: u8 = 0xE9;

/// Cabecera de imagen de Espressif (24 bytes, campos principales).
#[repr(C, packed)]
pub struct EspImageHeader {
    pub magic: u8,          // 0xE9
    pub segment_count: u8,  // número de segmentos que siguen
    pub spi_mode: u8,       // QIO/QOUT/DIO/DOUT
    pub spi_speed_size: u8, // nibble alto = tamaño flash, bajo = velocidad
    pub entry_addr: u32,    // dirección de entrada del programa
    // ... (campos extendidos: chip_id, hash_appended, etc.)
}

/// Cabecera de cada segmento que la ROM copia a su dirección de carga.
#[repr(C, packed)]
pub struct EspSegmentHeader {
    pub load_addr: u32, // destino en IRAM/DRAM
    pub length: u32,    // bytes del segmento
}

// TODO(fase-bootloader): parseo y validación de segmentos + hash SHA-256.
