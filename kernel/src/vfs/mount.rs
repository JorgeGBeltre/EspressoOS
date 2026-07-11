//! Tabla de puntos de montaje y resolución/normalización de rutas.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Asocia rutas absolutas (`/`, `/dev`, `/tmp`) con implementaciones de
//! [`FileSystem`]. La resolución de una ruta cruza montajes eligiendo el punto
//! de montaje con el prefijo de componentes más largo, y después camina por los
//! directorios del FS con `Inode::lookup`.
//!
//! La normalización de rutas es estilo Unix y totalmente pura (testeable):
//!  - Requiere ruta absoluta (empieza por `/`).
//!  - Colapsa barras repetidas (`//` -> `/`).
//!  - Elimina componentes `.`.
//!  - Resuelve `..` retrocediendo un componente (en la raíz, `..` es la raíz).
#![allow(dead_code)]

use crate::prelude::*;
use crate::arch::xtensa::sync::Mutex;
use super::inode::{FileSystem, Inode};

/// Longitud máxima de un componente de nombre (bytes).
pub const MAX_NAME_LEN: usize = 255;

/// Un punto de montaje: ruta normalizada -> FS.
struct MountPoint {
    /// Ruta absoluta normalizada donde está montado (p. ej. "/", "/dev").
    path: String,
    /// Sistema de archivos montado.
    fs: Arc<dyn FileSystem>,
}

/// Tabla global de montajes. Protegida por `Mutex` (previsión SMP/ISR).
static MOUNTS: Mutex<Vec<MountPoint>> = Mutex::new(Vec::new());

/// Normaliza una ruta absoluta estilo Unix.
///
/// Devuelve la forma canónica (siempre empieza por `/`, sin barras repetidas,
/// sin `.` ni `..`). La raíz normaliza a `"/"`.
///
/// Errores:
///  - [`KError::InvalidArgument`] si la ruta no es absoluta.
///  - [`KError::NameTooLong`] si algún componente excede [`MAX_NAME_LEN`].
pub fn normalize(path: &str) -> KResult<String> {
    if !path.starts_with('/') {
        return Err(KError::InvalidArgument);
    }

    let mut comps: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            // Barra repetida o componente vacío, y "." -> se ignoran.
            "" | "." => {}
            // ".." retrocede un componente; en la raíz no hay a dónde subir.
            ".." => {
                comps.pop();
            }
            name => {
                if name.len() > MAX_NAME_LEN {
                    return Err(KError::NameTooLong);
                }
                comps.push(name);
            }
        }
    }

    if comps.is_empty() {
        return Ok(String::from("/"));
    }

    let mut out = String::new();
    for c in comps.iter() {
        out.push('/');
        out.push_str(c);
    }
    Ok(out)
}

/// Divide una ruta YA NORMALIZADA en `(directorio_padre, nombre_final)`.
///
/// `path` no puede ser `"/"` (la raíz no tiene padre). Ejemplos:
///  - `"/foo/bar"` -> `("/foo", "bar")`
///  - `"/foo"`     -> `("/", "foo")`
pub fn split_parent(path: &str) -> KResult<(&str, &str)> {
    if path.is_empty() || path == "/" {
        return Err(KError::InvalidArgument);
    }
    match path.rfind('/') {
        // El único '/' está al principio: el padre es la raíz.
        Some(0) => Ok(("/", &path[1..])),
        Some(i) => {
            let name = &path[i + 1..];
            if name.is_empty() {
                // No debería ocurrir en una ruta normalizada.
                Err(KError::InvalidArgument)
            } else {
                Ok((&path[..i], name))
            }
        }
        // Sin '/': no es una ruta absoluta normalizada.
        None => Err(KError::InvalidArgument),
    }
}

/// Componentes no vacíos de una ruta normalizada. `"/"` -> vector vacío.
fn split_components(normalized: &str) -> Vec<&str> {
    normalized.split('/').filter(|s| !s.is_empty()).collect()
}

/// Monta `fs` en `path`. La ruta se normaliza antes de registrar.
///
/// Errores: [`KError::AlreadyExists`] si ya hay algo montado exactamente ahí.
pub fn mount(path: &str, fs: Arc<dyn FileSystem>) -> KResult<()> {
    let norm = normalize(path)?;
    let mut mounts = MOUNTS.lock();
    if mounts.iter().any(|m| m.path == norm) {
        return Err(KError::AlreadyExists);
    }
    mounts.push(MountPoint { path: norm, fs });
    Ok(())
}

/// Desmonta el FS montado exactamente en `path`.
///
/// Errores: [`KError::NotFound`] si no hay montaje en esa ruta.
pub fn unmount(path: &str) -> KResult<()> {
    let norm = normalize(path)?;
    let mut mounts = MOUNTS.lock();
    let before = mounts.len();
    mounts.retain(|m| m.path != norm);
    if mounts.len() == before {
        Err(KError::NotFound)
    } else {
        Ok(())
    }
}

/// Resuelve una ruta absoluta a un inodo, cruzando montajes.
///
/// Elige el punto de montaje cuyo conjunto de componentes es el prefijo más
/// largo de la ruta objetivo, y camina el resto con `Inode::lookup`.
pub fn resolve(path: &str) -> KResult<Arc<dyn Inode>> {
    let norm = normalize(path)?;
    let target: Vec<&str> = split_components(&norm);

    // Selección del mejor montaje (prefijo de componentes más largo).
    let (fs, mount_len) = {
        let mounts = MOUNTS.lock();
        let mut best: Option<(Arc<dyn FileSystem>, usize)> = None;
        for m in mounts.iter() {
            let mc = split_components(&m.path);
            let mlen = mc.len();
            if mlen > target.len() {
                continue;
            }
            // ¿Son los primeros `mlen` componentes del objetivo iguales a `mc`?
            let is_prefix = mc.iter().zip(target.iter()).all(|(a, b)| a == b);
            if is_prefix {
                let replace = match &best {
                    Some((_, best_len)) => mlen > *best_len,
                    None => true,
                };
                if replace {
                    best = Some((m.fs.clone(), mlen));
                }
            }
        }
        match best {
            Some(v) => v,
            None => return Err(KError::NotFound),
        }
    };

    // Camina los componentes restantes desde la raíz del FS elegido.
    let mut node = fs.root();
    for comp in target.iter().skip(mount_len) {
        node = node.lookup(comp)?; // `comp: &&str` coacciona a `&str`
    }
    Ok(node)
}

// ---------------------------------------------------------------------------
// Tests puros de normalización/partición de rutas.
//
// Nota: el kernel es `#![no_std] #![no_main]`, por lo que estos tests sólo se
// compilan/ejecutan bajo un arnés de test de host (`cfg(test)`). Son inertes
// para la compilación del firmware. Cubren la lógica pura de rutas.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliza_raiz() {
        assert_eq!(normalize("/").unwrap(), "/");
        assert_eq!(normalize("///").unwrap(), "/");
        assert_eq!(normalize("/.").unwrap(), "/");
        assert_eq!(normalize("/..").unwrap(), "/");
        assert_eq!(normalize("/../..").unwrap(), "/");
    }

    #[test]
    fn colapsa_barras_y_puntos() {
        assert_eq!(normalize("/a//b").unwrap(), "/a/b");
        assert_eq!(normalize("/a/./b").unwrap(), "/a/b");
        assert_eq!(normalize("/a/b/").unwrap(), "/a/b");
        assert_eq!(normalize("//a///b////c//").unwrap(), "/a/b/c");
    }

    #[test]
    fn resuelve_punto_punto() {
        assert_eq!(normalize("/a/../b").unwrap(), "/b");
        assert_eq!(normalize("/a/b/../c").unwrap(), "/a/c");
        assert_eq!(normalize("/a/b/../../c").unwrap(), "/c");
        assert_eq!(normalize("/a/../../b").unwrap(), "/b");
        assert_eq!(normalize("/dev/../tmp/./x").unwrap(), "/tmp/x");
    }

    #[test]
    fn rechaza_relativas() {
        assert_eq!(normalize("a/b"), Err(KError::InvalidArgument));
        assert_eq!(normalize(""), Err(KError::InvalidArgument));
        assert_eq!(normalize("./x"), Err(KError::InvalidArgument));
    }

    #[test]
    fn nombre_demasiado_largo() {
        // 256 caracteres 'a' -> excede MAX_NAME_LEN (255).
        let mut largo = String::from("/");
        for _ in 0..(MAX_NAME_LEN + 1) {
            largo.push('a');
        }
        assert_eq!(normalize(&largo), Err(KError::NameTooLong));
    }

    #[test]
    fn split_parent_casos() {
        assert_eq!(split_parent("/foo/bar").unwrap(), ("/foo", "bar"));
        assert_eq!(split_parent("/foo").unwrap(), ("/", "foo"));
        assert_eq!(split_parent("/"), Err(KError::InvalidArgument));
    }

    #[test]
    fn split_components_casos() {
        assert!(split_components("/").is_empty());
        assert_eq!(split_components("/a/b/c"), alloc::vec!["a", "b", "c"]);
    }
}
