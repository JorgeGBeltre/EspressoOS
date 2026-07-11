//! Sistema de archivos en RAM (`ramfs`) para `/tmp`. — Fase 4.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Implementación REAL y completa de un FS volátil: un árbol de directorios y
//! archivos que vive en el heap del kernel. Se pierde al reiniciar. Sirve como
//! primer FS de prueba del VFS (antes de tener la flash operativa) y como
//! backing de `/tmp`.
//!
//! Diseño:
//! - Cada nodo (`RamNode`) es un archivo (bytes en un `Vec<u8>`) o un
//!   directorio (mapa ordenado `nombre -> hijo`).
//! - Los nodos se comparten como `Arc<RamNode>` y se exponen al VFS como
//!   `Arc<dyn Inode>`.
//! - La mutabilidad interior (los métodos del trait toman `&self`) se protege
//!   con `arch::xtensa::sync::Mutex`, según §0.7 y §3.2.4 del contrato.
#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;
use crate::vfs::inode::{DirEntry, FileSystem, FsStat, Inode, InodeKind};
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU32, Ordering};

/// Contador global de números de inodo. Monótono y único durante la vida del
/// kernel; basta para diagnóstico (`ls -i`, `readdir`) dado que `ramfs` es
/// volátil. Se usa `AtomicU32` (no `U64`) porque la LX7 es de 32 bits y no
/// garantiza atómicos de 64 bits nativos.
static NEXT_INO: AtomicU32 = AtomicU32::new(1);

/// Reserva un número de inodo nuevo.
fn alloc_ino() -> u64 {
    NEXT_INO.fetch_add(1, Ordering::Relaxed) as u64
}

/// Contenido mutable de un nodo, protegido por el `Mutex` del nodo.
enum RamBody {
    /// Archivo: bytes en el heap.
    File(Vec<u8>),
    /// Directorio: `nombre -> hijo`. `BTreeMap` da orden estable para `readdir`.
    Dir(BTreeMap<String, Arc<RamNode>>),
}

/// Nodo del árbol de `ramfs` (archivo o directorio).
struct RamNode {
    /// Número de inodo (fijo durante la vida del nodo).
    ino: u64,
    /// Tipo del nodo (fijo tras la creación).
    kind: InodeKind,
    /// Contenido mutable.
    body: Mutex<RamBody>,
}

impl RamNode {
    /// Crea un directorio vacío.
    fn new_dir() -> Arc<RamNode> {
        Arc::new(RamNode {
            ino: alloc_ino(),
            kind: InodeKind::Dir,
            body: Mutex::new(RamBody::Dir(BTreeMap::new())),
        })
    }

    /// Crea un archivo vacío.
    fn new_file() -> Arc<RamNode> {
        Arc::new(RamNode {
            ino: alloc_ino(),
            kind: InodeKind::File,
            body: Mutex::new(RamBody::File(Vec::new())),
        })
    }
}

impl Inode for RamNode {
    fn kind(&self) -> InodeKind {
        self.kind
    }

    fn size(&self) -> u64 {
        let body = self.body.lock();
        match &*body {
            RamBody::File(data) => data.len() as u64,
            // Los directorios no tienen un tamaño en bytes significativo.
            RamBody::Dir(_) => 0,
        }
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let body = self.body.lock();
        match &*body {
            RamBody::File(data) => {
                // Un offset mayor que el archivo (o que cualquier `usize`) = EOF.
                let start = match usize::try_from(off) {
                    Ok(v) => v,
                    Err(_) => return Ok(0),
                };
                if start >= data.len() {
                    return Ok(0);
                }
                let disponible = data.len() - start; // start < len => sin underflow
                let n = core::cmp::min(disponible, buf.len());
                // Rangos comprobados: start + n <= len y n <= buf.len().
                buf[..n].copy_from_slice(&data[start..start + n]);
                Ok(n)
            }
            RamBody::Dir(_) => Err(KError::IsADirectory),
        }
    }

    fn write_at(&self, off: u64, buf: &[u8]) -> KResult<usize> {
        let mut body = self.body.lock();
        match &mut *body {
            RamBody::File(data) => {
                let start = usize::try_from(off).map_err(|_| KError::InvalidArgument)?;
                // Fin de la escritura; sin overflow.
                let end = start.checked_add(buf.len()).ok_or(KError::NoMem)?;
                if end > data.len() {
                    // Extiende con ceros hasta `end` (huecos = 0). Usamos
                    // `try_reserve` para NO abortar por OOM: devolvemos `NoMem`.
                    let extra = end - data.len();
                    data.try_reserve(extra).map_err(|_| KError::NoMem)?;
                    data.resize(end, 0);
                }
                // start..end tiene longitud buf.len(); rangos válidos.
                data[start..end].copy_from_slice(buf);
                Ok(buf.len())
            }
            RamBody::Dir(_) => Err(KError::IsADirectory),
        }
    }

    fn truncate(&self, len: u64) -> KResult<()> {
        let mut body = self.body.lock();
        match &mut *body {
            RamBody::File(data) => {
                let n = usize::try_from(len).map_err(|_| KError::InvalidArgument)?;
                if n > data.len() {
                    let extra = n - data.len();
                    data.try_reserve(extra).map_err(|_| KError::NoMem)?;
                }
                data.resize(n, 0);
                Ok(())
            }
            RamBody::Dir(_) => Err(KError::IsADirectory),
        }
    }

    fn readdir(&self, index: usize) -> KResult<Option<DirEntry>> {
        let body = self.body.lock();
        match &*body {
            RamBody::Dir(children) => {
                // Orden estable de `BTreeMap`. O(index) por llamada; el VFS itera
                // 0.. hasta `None`, coste O(n^2) en dirs grandes: aceptable para
                // `/tmp` (dirs pequeños). Documentado como posible mejora.
                match children.iter().nth(index) {
                    Some((name, node)) => Ok(Some(DirEntry {
                        name: name.clone(),
                        kind: node.kind,
                        ino: node.ino,
                    })),
                    None => Ok(None),
                }
            }
            RamBody::File(_) => Err(KError::NotADirectory),
        }
    }

    fn lookup(&self, name: &str) -> KResult<Arc<dyn Inode>> {
        let body = self.body.lock();
        match &*body {
            RamBody::Dir(children) => match children.get(name) {
                // Coerción `Arc<RamNode>` -> `Arc<dyn Inode>` vía anotación de tipo.
                Some(node) => {
                    let out: Arc<dyn Inode> = node.clone();
                    Ok(out)
                }
                None => Err(KError::NotFound),
            },
            RamBody::File(_) => Err(KError::NotADirectory),
        }
    }

    fn create(&self, name: &str, kind: InodeKind) -> KResult<Arc<dyn Inode>> {
        let mut body = self.body.lock();
        match &mut *body {
            RamBody::Dir(children) => {
                if children.contains_key(name) {
                    return Err(KError::AlreadyExists);
                }
                let node = match kind {
                    InodeKind::File => RamNode::new_file(),
                    InodeKind::Dir => RamNode::new_dir(),
                    // `ramfs` no crea dispositivos ni enlaces simbólicos propios;
                    // los dispositivos viven en `devfs`.
                    InodeKind::Device | InodeKind::Symlink => return Err(KError::NotSupported),
                };
                children.insert(String::from(name), node.clone());
                let out: Arc<dyn Inode> = node;
                Ok(out)
            }
            RamBody::File(_) => Err(KError::NotADirectory),
        }
    }

    fn unlink(&self, name: &str) -> KResult<()> {
        let mut body = self.body.lock();
        match &mut *body {
            RamBody::Dir(children) => {
                // Comprobamos existencia y, si es directorio, que esté vacío.
                // El bloqueo del hijo se toma y libera ANTES de mutar el padre,
                // manteniendo el orden padre->hijo (sin deadlocks en el árbol).
                let vacio = match children.get(name) {
                    None => return Err(KError::NotFound),
                    Some(node) => {
                        let child_body = node.body.lock();
                        match &*child_body {
                            RamBody::Dir(sub) => sub.is_empty(),
                            RamBody::File(_) => true,
                        }
                    }
                };
                if !vacio {
                    // No hay variante ENOTEMPTY en `KError`; `Busy` comunica
                    // "no se puede borrar: directorio no vacío".
                    return Err(KError::Busy);
                }
                // Al quitarlo del mapa se suelta el `Arc`; si nadie más lo
                // referencia (p. ej. un `OpenFile` abierto), el subárbol se
                // libera. Semántica tipo `unlink` de Unix.
                children.remove(name);
                Ok(())
            }
            RamBody::File(_) => Err(KError::NotADirectory),
        }
    }

    // `sync` usa la implementación por defecto del trait (`Ok(())`): `ramfs`
    // es volátil, no hay nada que persistir.
}

/// Sistema de archivos en RAM. Se monta típicamente en `/tmp`.
pub struct RamFs {
    /// Directorio raíz del FS.
    root: Arc<RamNode>,
}

impl RamFs {
    /// Crea un `ramfs` vacío (solo la raíz) listo para `vfs::mount`. [CANÓNICO]
    pub fn new() -> Arc<RamFs> {
        Arc::new(RamFs {
            root: RamNode::new_dir(),
        })
    }

    /// Suma recursiva de los bytes de todos los archivos del subárbol.
    ///
    /// Recorre el árbol bloqueando cada nodo en orden padre->hijo. La recursión
    /// es proporcional a la profundidad del árbol; para `/tmp` (poco profundo)
    /// es seguro. `saturating_add` evita cualquier overflow de contador.
    fn used_bytes(node: &Arc<RamNode>) -> u64 {
        let body = node.body.lock();
        match &*body {
            RamBody::File(data) => data.len() as u64,
            RamBody::Dir(children) => {
                let mut total: u64 = 0;
                for (_, child) in children.iter() {
                    total = total.saturating_add(Self::used_bytes(child));
                }
                total
            }
        }
    }
}

impl FileSystem for RamFs {
    fn name(&self) -> &str {
        "ramfs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        let r: Arc<dyn Inode> = self.root.clone();
        r
    }

    fn sync(&self) -> KResult<()> {
        // Volátil: nada que sincronizar.
        Ok(())
    }

    fn stat(&self) -> FsStat {
        // `ramfs` no tiene capacidad fija: el límite real es el heap. Reportamos
        // `total = usado` (todo lo asignado está en uso) y `block_size = 1`
        // porque no trabaja por bloques. Un consumidor que quiera la capacidad
        // real de memoria debe mirar `mm::stats()`.
        let usado = RamFs::used_bytes(&self.root);
        FsStat {
            total_bytes: usado,
            used_bytes: usado,
            block_size: 1,
        }
    }
}
