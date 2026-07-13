//! Prelude del kernel: tipos y alias compartidos por todos los subsistemas.
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

pub use alloc::boxed::Box;
pub use alloc::string::String;
pub use alloc::sync::Arc;
pub use alloc::vec::Vec;

/// Error canónico del kernel. TODO subsistema devuelve esto (o un alias suyo).
/// No añadir variantes sin actualizar el contrato.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KError {
    /// Sin memoria (allocator o tabla llena de otra naturaleza -> usar TableFull).
    NoMem,
    /// Recurso, ruta o entrada inexistente.
    NotFound,
    /// Ya existe (crear algo que colisiona).
    AlreadyExists,
    /// Se esperaba directorio y no lo es.
    NotADirectory,
    /// Se esperaba archivo y es directorio.
    IsADirectory,
    /// Argumento inválido (rango, formato, alineación).
    InvalidArgument,
    /// Operación no permitida por permisos/estado.
    PermissionDenied,
    /// No implementado / no soportado por este backend.
    NotSupported,
    /// La operación bloquearía y se pidió no-bloqueante.
    WouldBlock,
    /// Recurso ocupado (lock tomado, dispositivo en uso).
    Busy,
    /// Error de E/S de bajo nivel (bus, flash).
    IoError,
    /// Descriptor de archivo inválido.
    BadFd,
    /// Nombre o ruta demasiado largos.
    NameTooLong,
    /// Sin espacio (FS/partición llenos).
    NoSpace,
    /// Datos corruptos (FS/flash/imagen).
    Corrupt,
    /// Tiempo de espera agotado.
    Timeout,
    /// Puntero/dirección inválidos (fault de acceso).
    Fault,
    /// Tabla interna llena (tareas, descriptores, montajes).
    TableFull,
}

impl KError {
    /// Traducción a errno negativo para el ABI de syscalls (§8).
    /// Convención: el retorno de syscall es `isize`; error = -errno.
    pub const fn as_errno(self) -> isize {
        match self {
            KError::NotFound => -2,           // ENOENT
            KError::IoError => -5,            // EIO
            KError::BadFd => -9,              // EBADF
            KError::NoMem => -12,             // ENOMEM
            KError::PermissionDenied => -13,  // EACCES
            KError::Fault => -14,             // EFAULT
            KError::Busy => -16,              // EBUSY
            KError::AlreadyExists => -17,     // EEXIST
            KError::NotADirectory => -20,     // ENOTDIR
            KError::IsADirectory => -21,      // EISDIR
            KError::InvalidArgument => -22,   // EINVAL
            KError::TableFull => -23,         // ENFILE (aprox.)
            KError::NoSpace => -28,           // ENOSPC
            KError::NameTooLong => -36,       // ENAMETOOLONG
            KError::WouldBlock => -11,        // EAGAIN
            KError::Timeout => -110,          // ETIMEDOUT
            KError::NotSupported => -95,      // ENOTSUP
            KError::Corrupt => -84,           // EILSEQ (aprox.)
        }
    }
}

/// Resultado canónico del kernel. Firma obligatoria en toda API interna.
pub type KResult<T> = Result<T, KError>;

/// Constantes de layout de flash/RAM compartidas (§4). Fuente única.
pub mod layout {
    // Flash 16 MB. Offsets/tamaños DEBEN coincidir con partitions.csv.
    pub const FLASH_SIZE: u32 = 0x0100_0000; // 16 MB
    pub const PART_TABLE_OFFSET: u32 = 0x0000_8000;
    pub const NVS_OFFSET: u32 = 0x0000_9000;
    pub const NVS_SIZE: u32 = 0x0000_6000; // 24 KB
    pub const OTADATA_OFFSET: u32 = 0x0000_F000;
    pub const OTADATA_SIZE: u32 = 0x0000_2000; // 8 KB
    pub const FACTORY_OFFSET: u32 = 0x0002_0000; // slot A (kernel)
    pub const FACTORY_SIZE: u32 = 0x0040_0000; // 4 MB
    pub const OTA0_OFFSET: u32 = 0x0042_0000; // slot B
    pub const OTA0_SIZE: u32 = 0x0040_0000; // 4 MB
    pub const FS_OFFSET: u32 = 0x0082_0000; // LittleFS
    pub const FS_SIZE: u32 = 0x007D_0000; // ~7.8 MB
    pub const COREDUMP_OFFSET: u32 = 0x00FF_0000;
    pub const COREDUMP_SIZE: u32 = 0x0001_0000; // 64 KB

    pub const FLASH_SECTOR_SIZE: usize = 4096; // borrado mínimo

    // RAM.
    // SRAM interna (Fase 0/1). Subido 64->128 KB para dar holgura a esp-wifi, que
    // reserva decenas de KB de estructuras internas en el arranque de la radio.
    // Como la región interna se registra ANTES que la PSRAM, las asignaciones
    // pequeñas (incluidas las potencialmente DMA de esp-wifi) tienden a caer aquí.
    pub const KERNEL_HEAP_SIZE: usize = 128 * 1024;
    pub const PSRAM_SIZE: usize = 8 * 1024 * 1024;
    pub const DEFAULT_STACK_SIZE: usize = 8 * 1024; // pila por tarea (Fase 2)
}
