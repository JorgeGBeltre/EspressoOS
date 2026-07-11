//! Sistema de archivos de dispositivos `/dev` (DevFs).
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Expone los drivers como nodos siguiendo la filosofía "todo-es-un-archivo".
//! Un [`Device`] es la abstracción mínima (read/write/ioctl); DevFs lo envuelve
//! en un [`Inode`] de tipo [`InodeKind::Device`]. La raíz de DevFs es un
//! directorio virtual que lista los dispositivos registrados.
//!
//! Dispositivos base creados por [`init`]:
//!  - `/dev/null`    — descarta escrituras, lecturas dan EOF (0 bytes).
//!  - `/dev/zero`    — lecturas rellenan con ceros, escrituras se descartan.
//!  - `/dev/console` — delega en `drivers::uart` (consola USB-Serial-JTAG).
#![allow(dead_code)]

use crate::prelude::*;
use crate::arch::xtensa::sync::Mutex;
use super::inode::{DirEntry, FileSystem, FsStat, Inode, InodeKind};
use super::mount::MAX_NAME_LEN;

/// Un dispositivo de carácter/bloque expuesto en `/dev`.
pub trait Device: Send + Sync {
    /// Lee desde el dispositivo. `off` puede ignorarse en dispositivos de flujo.
    fn read(&self, off: u64, buf: &mut [u8]) -> KResult<usize>;
    /// Escribe en el dispositivo. Devuelve bytes aceptados.
    fn write(&self, off: u64, buf: &[u8]) -> KResult<usize>;
    /// Control de dispositivo. Por defecto: no soportado.
    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<usize> {
        let _ = (cmd, arg);
        Err(KError::NotSupported)
    }
}

/// Tabla compartida nombre -> dispositivo. Compartida entre el FS y su raíz.
type DevTable = Arc<Mutex<Vec<(String, Arc<dyn Device>)>>>;

/// FS especial que expone los dispositivos registrados. Montar en `/dev`.
pub struct DevFs {
    /// Tabla de dispositivos registrados.
    table: DevTable,
    /// Inodo raíz (directorio virtual) que referencia la tabla.
    root: Arc<DevRoot>,
}

impl DevFs {
    /// Crea un DevFs vacío listo para montar.
    pub fn new() -> Arc<DevFs> {
        let table: DevTable = Arc::new(Mutex::new(Vec::new()));
        let root = Arc::new(DevRoot {
            table: table.clone(),
        });
        Arc::new(DevFs { table, root })
    }
}

impl FileSystem for DevFs {
    fn name(&self) -> &str {
        "devfs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sync(&self) -> KResult<()> {
        Ok(())
    }

    fn stat(&self) -> FsStat {
        // DevFs no consume almacenamiento persistente.
        FsStat::default()
    }
}

/// Inodo raíz de DevFs: directorio virtual sobre la tabla de dispositivos.
struct DevRoot {
    table: DevTable,
}

impl Inode for DevRoot {
    fn kind(&self) -> InodeKind {
        InodeKind::Dir
    }

    fn size(&self) -> u64 {
        0
    }

    fn read_at(&self, _off: u64, _buf: &mut [u8]) -> KResult<usize> {
        Err(KError::IsADirectory)
    }

    fn write_at(&self, _off: u64, _buf: &[u8]) -> KResult<usize> {
        Err(KError::IsADirectory)
    }

    fn readdir(&self, index: usize) -> KResult<Option<DirEntry>> {
        let table = self.table.lock();
        match table.get(index) {
            Some((name, _)) => Ok(Some(DirEntry {
                name: name.clone(),
                kind: InodeKind::Device,
                ino: (index as u64).saturating_add(1),
            })),
            None => Ok(None),
        }
    }

    fn lookup(&self, name: &str) -> KResult<Arc<dyn Inode>> {
        let table = self.table.lock();
        for (n, dev) in table.iter() {
            if n == name {
                return Ok(Arc::new(DevNode { dev: dev.clone() }));
            }
        }
        Err(KError::NotFound)
    }

    fn create(&self, _name: &str, _kind: InodeKind) -> KResult<Arc<dyn Inode>> {
        // Los nodos de /dev se registran vía `register`, no se crean por VFS.
        Err(KError::PermissionDenied)
    }

    fn unlink(&self, _name: &str) -> KResult<()> {
        Err(KError::PermissionDenied)
    }
}

/// Inodo que envuelve un [`Device`] concreto (nodo `/dev/<name>`).
struct DevNode {
    dev: Arc<dyn Device>,
}

impl Inode for DevNode {
    fn kind(&self) -> InodeKind {
        InodeKind::Device
    }

    fn size(&self) -> u64 {
        0
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        self.dev.read(off, buf)
    }

    fn write_at(&self, off: u64, buf: &[u8]) -> KResult<usize> {
        self.dev.write(off, buf)
    }

    fn truncate(&self, _len: u64) -> KResult<()> {
        // Los dispositivos ignoran el truncado (permite abrir con TRUNC).
        Ok(())
    }
    // readdir/lookup/create/unlink usan el default (NotADirectory).
}

/// Registra un dispositivo bajo `name` (aparece como `/dev/<name>`).
///
/// Errores: [`KError::AlreadyExists`] si el nombre ya existe;
/// [`KError::NameTooLong`] si excede [`MAX_NAME_LEN`].
pub fn register(devfs: &Arc<DevFs>, name: &str, dev: Arc<dyn Device>) -> KResult<()> {
    if name.len() > MAX_NAME_LEN {
        return Err(KError::NameTooLong);
    }
    let mut table = devfs.table.lock();
    if table.iter().any(|(n, _)| n == name) {
        return Err(KError::AlreadyExists);
    }
    table.push((String::from(name), dev));
    Ok(())
}

/// Crea el DevFs y registra los dispositivos base (`null`, `zero`, `console`).
pub fn init() -> KResult<Arc<DevFs>> {
    let devfs = DevFs::new();
    register(&devfs, "null", Arc::new(NullDevice))?;
    register(&devfs, "zero", Arc::new(ZeroDevice))?;
    register(&devfs, "console", Arc::new(ConsoleDevice))?;
    Ok(devfs)
}

// --------------------------- Dispositivos base -----------------------------

/// `/dev/null`: descarta escrituras; las lecturas devuelven EOF.
struct NullDevice;
impl Device for NullDevice {
    fn read(&self, _off: u64, _buf: &mut [u8]) -> KResult<usize> {
        Ok(0)
    }
    fn write(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        Ok(buf.len())
    }
}

/// `/dev/zero`: lecturas rellenan con ceros; escrituras se descartan.
struct ZeroDevice;
impl Device for ZeroDevice {
    fn read(&self, _off: u64, buf: &mut [u8]) -> KResult<usize> {
        for b in buf.iter_mut() {
            *b = 0;
        }
        Ok(buf.len())
    }
    fn write(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        Ok(buf.len())
    }
}

/// `/dev/console`: delega en el driver de consola (`drivers::uart`).
///
/// RIESGO/ASUNCIÓN: usa `drivers::uart::{read, write}` (contrato §3.9). En Fase 0
/// `write` puede apoyarse en esp-println y `read` no bloquea (0 si no hay datos).
struct ConsoleDevice;
impl Device for ConsoleDevice {
    fn read(&self, _off: u64, buf: &mut [u8]) -> KResult<usize> {
        Ok(crate::drivers::uart::read(buf))
    }
    fn write(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        Ok(crate::drivers::uart::write(buf))
    }
}
