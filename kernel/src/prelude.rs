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
    // 16K, no 8K. En Xtensa la excepción/syscall corre sobre la pila de la tarea
    // interrumpida, así que la pila de una tarea de usuario sostiene sus frames MÁS los
    // del kernel durante un syscall -- y `spawn` (load_elf + relocación + write_argv +
    // register_process) es el camino más profundo. Con 8K, init desbordaba justo ahí,
    // pisaba sus propios slots de spill de registros, y el crash parecía un bug de
    // context-switch (no lo era: el scheduler preserva registros, verificado en hardware).
    // 16K iguala la pila del net task y cubre el path de spawn con margen razonable.
    #[cfg(not(feature = "diag-32k-stack"))]
    pub const DEFAULT_STACK_SIZE: usize = 16 * 1024;
    // DIAGNÓSTICO (experimento de pila BLE): solo bajo la feature `diag-32k-stack`, sube la
    // pila de userland a 32K para el brazo B del A/B. El `default` NO la activa → 16K queda
    // garantizado por el compilador; el 32K no puede viajar a una imagen de ship (invariante
    // estructural, no "acordarse de revertir"). Afecta a TODOS los procesos de userland (no
    // solo /bin/ble), lo cual para el experimento da igual: la variable que arbitra es la
    // pila del llamador. `net_task` no se ve afectado (usa NET_STACK_SIZE, no este const).
    #[cfg(feature = "diag-32k-stack")]
    pub const DEFAULT_STACK_SIZE: usize = 32 * 1024;
}
