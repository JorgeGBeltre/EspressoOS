#![allow(dead_code)]

use crate::prelude::*;
use crate::arch::xtensa::sync::Mutex;
use super::inode::{FileSystem, Inode};

pub const MAX_NAME_LEN: usize = 255;

struct MountPoint {

    path: String,

    fs: Arc<dyn FileSystem>,
}

static MOUNTS: Mutex<Vec<MountPoint>> = Mutex::new(Vec::new());

pub fn normalize(path: &str) -> KResult<String> {
    if !path.starts_with('/') {
        return Err(KError::InvalidArgument);
    }

    let mut comps: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {

            "" | "." => {}

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

pub fn split_parent(path: &str) -> KResult<(&str, &str)> {
    if path.is_empty() || path == "/" {
        return Err(KError::InvalidArgument);
    }
    match path.rfind('/') {

        Some(0) => Ok(("/", &path[1..])),
        Some(i) => {
            let name = &path[i + 1..];
            if name.is_empty() {

                Err(KError::InvalidArgument)
            } else {
                Ok((&path[..i], name))
            }
        }

        None => Err(KError::InvalidArgument),
    }
}

fn split_components(normalized: &str) -> Vec<&str> {
    normalized.split('/').filter(|s| !s.is_empty()).collect()
}

pub fn mount(path: &str, fs: Arc<dyn FileSystem>) -> KResult<()> {
    let norm = normalize(path)?;
    let mut mounts = MOUNTS.lock();
    if mounts.iter().any(|m| m.path == norm) {
        return Err(KError::AlreadyExists);
    }
    mounts.push(MountPoint { path: norm, fs });
    Ok(())
}

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

pub fn resolve(path: &str) -> KResult<Arc<dyn Inode>> {
    let norm = normalize(path)?;
    let target: Vec<&str> = split_components(&norm);

    let (fs, mount_len) = {
        let mounts = MOUNTS.lock();
        let mut best: Option<(Arc<dyn FileSystem>, usize)> = None;
        for m in mounts.iter() {
            let mc = split_components(&m.path);
            let mlen = mc.len();
            if mlen > target.len() {
                continue;
            }

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

    let mut node = fs.root();
    for comp in target.iter().skip(mount_len) {
        node = node.lookup(comp)?;
    }
    Ok(node)
}

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
