//! Punto de entrada del bootloader de 2ª etapa (ESQUELETO).
//!
//! Responsabilidad futura: inicializar reloj/PSRAM mínimos, leer la tabla de
//! particiones, seleccionar el slot de arranque (factory/ota_0 según OTA),
//! verificar y cargar la imagen del kernel a IRAM/DRAM y saltar a su entrada.
//!
//! En la Fase 0 este binario NO se compila (crate fuera del workspace); el
//! arranque lo cubre la ROM + esp-hal. Se conserva para materializar la
//! arquitectura del árbol y desarrollarlo en su fase.
#![no_std]
#![no_main]

mod flash;
mod multiboot2;
mod partition_table;
mod uart;

// Sin panic handler / _start reales todavía: se añaden al activar el crate.
