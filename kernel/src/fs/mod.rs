//! Sistemas de archivos concretos. — Fase 4.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Implementaciones concretas que se montan en el VFS (`crate::vfs`):
//!
//! - [`ramfs::RamFs`]: FS volátil en heap (árbol de dirs/archivos en RAM).
//!   Implementación real y completa. Se monta en `/tmp` y sirve como primer FS
//!   de prueba del VFS.
//! - [`littlefs::LittleFs`]: binding (borrador) de LittleFS sobre
//!   `drivers::flash`, montable en `/`. Persistente y resistente a cortes de
//!   energía. El adaptador de bloques es real; el núcleo `lfs` queda pendiente
//!   de enlazar una crate externa (ver `needs_crates`).
//!
//! Ambos implementan el trait `crate::vfs::inode::FileSystem` del contrato.
#![allow(dead_code)]

pub mod littlefs;
pub mod ramfs;

// Re-exports de conveniencia: el resto del kernel usa `fs::RamFs` / `fs::LittleFs`.
pub use littlefs::LittleFs;
pub use ramfs::RamFs;
