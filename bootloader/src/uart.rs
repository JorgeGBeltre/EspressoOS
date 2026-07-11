//! Driver UART mínimo del bootloader, sin HAL (ESQUELETO).
//!
//! Acceso directo a los registros de UART0 para emitir trazas tempranas de
//! arranque antes de que exista cualquier abstracción. Se implementa a nivel
//! de registro para no depender de esp-hal en esta capa.
#![allow(dead_code)]

/// Emite un byte por UART0 (bloqueante). ESQUELETO.
pub fn putc(_b: u8) {
    // TODO(fase-bootloader): esperar TX FIFO y escribir en UART0_FIFO_REG.
}

/// Emite una cadena por UART0. ESQUELETO.
pub fn puts(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}
