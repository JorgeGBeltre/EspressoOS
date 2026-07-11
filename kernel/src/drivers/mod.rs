//! Drivers de dispositivos (ESQUELETO — Fase 3+).
//!
//! Acceso a periféricos del ESP32-S3. Parten de esp-hal y se sustituyen por
//! acceso a registro donde el control/aprendizaje lo justifique. Se exponen al
//! resto del kernel vía el VFS (`/dev`).
pub mod flash;
pub mod gpio;
pub mod i2c;
pub mod spi;
pub mod uart;
pub mod wifi;
