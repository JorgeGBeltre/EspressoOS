#![allow(dead_code)]

use super::inode::{FileSystem, Inode};
use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;

pub const MAX_NAME_LEN: usize = 255;

struct MountPoint {
    path: String,

    fs: Arc<dyn FileSystem>,
}

static MOUNTS: Mutex<Vec<MountPoint>> = Mutex::new(Vec::new());

/// Collapses `path` lexically against `base`, which must be absolute or empty.
///
/// Pure on purpose: no locks, no process, no filesystem. That separates what a path
/// MEANS from who is asking, and it is the half that tools/tests/logic_tests.py can
/// mirror -- the `mod tests` below cannot run at all, because the kernel is a no_std
/// binary with no lib target, so the Python harness is the only place this logic is
/// ever actually executed.
///
/// An absolute `path` ignores `base` entirely. That is checked here rather than left as
/// a precondition on the caller, so the function cannot be called wrong: handing it a
/// base and an absolute path is not an error, it just means what it should.
fn normalize_against(base: &str, path: &str) -> KResult<String> {
    // Before anything else. An empty path is not a relative path -- it names nothing at
    // all -- and without this it would fall through to the join and come out as `base`,
    // which is to say the caller's own working directory. That is reachable from
    // userland holding no valid pointer whatsoever: user_slice returns an empty slice
    // for len == 0 before validate_user ever runs, so `unlink(anything, 0)` arrives
    // here as "", and a zero-length path would delete the caller's cwd.
    if path.is_empty() {
        return Err(KError::InvalidArgument);
    }

    let base = if path.starts_with('/') { "" } else { base };

    let mut comps: Vec<&str> = Vec::new();
    // The base's components first, then the path's. That is what "relative to" means,
    // and it is what lets ".." pop across the join: base /tmp/x with path ../y is /tmp/y.
    for part in base.split('/').chain(path.split('/')) {
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

/// Turns any path the kernel is handed into an absolute, normalized one, resolving a
/// relative one against the calling process's working directory.
///
/// This lives here, and not in `resolve`, because here is the only place all the path
/// entry points share. `vfs::create_path` and `vfs::unlink` call normalize DIRECTLY and
/// never touch `resolve`, so a cwd applied in `resolve` would have left `open("foo")`
/// working while `open("foo", O_CREAT)` and `unlink("foo")` returned EINVAL from the
/// same directory.
///
/// It used to be every caller's job to prefix the cwd. The kernel shell did it; userland
/// could not, there being no getcwd -- which is why `cd /tmp; ls` listed /tmp and
/// `cd /tmp; /bin/ls` listed /. A rule every caller must remember is not an invariant.
///
/// `cwd_of_caller`, not `cwd_get`: a task with no process has no working directory, and
/// answering "/" would be a silent lie -- a relative path from the net task would read
/// some file under the root instead of failing. Those tasks pass absolute literals and
/// go on getting exactly the InvalidArgument they get today.
///
/// No caller may hold a lock across this: it takes SCHED and PROCESS_TABLE. Every one in
/// the tree already normalizes before locking -- `resolve` below at :97 before MOUNTS at
/// :101, `mount`, `unmount`, `vfs::open` before the fd table, and `cwd_set` before
/// PROCESS_TABLE, where getting it wrong would be same-lock reentry and a silent wedge.
pub fn normalize(path: &str) -> KResult<String> {
    if path.starts_with('/') {
        return normalize_against("", path);
    }
    // Relative -- and "" lands here too, since it does not start with '/'. It is left to
    // normalize_against to reject rather than guarded again here: the cost is one wasted
    // cwd lookup on exactly one input, which is cheaper than a second copy of the rule
    // that would have to stay in step with the first.
    let cwd = crate::scheduler::process::cwd_of_caller().ok_or(KError::InvalidArgument)?;
    normalize_against(&cwd, path)
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

// These cannot run: the kernel is a no_std binary with no lib target, so
// `cargo test -p espressoos-kernel --lib` answers "no library targets found" and this
// module is never compiled, let alone executed. tools/tests/logic_tests.py is a
// line-by-line Python port of normalize_against and is the only thing that actually
// exercises this logic -- keep the two in step, and treat the Python one as the test.
//
// They are written against normalize_against rather than normalize because that is the
// half with no scheduler under it. `normalize` cannot be tested here at any price: it
// asks the process table who is calling.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliza_raiz() {
        assert_eq!(normalize_against("", "/").unwrap(), "/");
        assert_eq!(normalize_against("", "///").unwrap(), "/");
        assert_eq!(normalize_against("", "/.").unwrap(), "/");
        assert_eq!(normalize_against("", "/..").unwrap(), "/");
        assert_eq!(normalize_against("", "/../..").unwrap(), "/");
    }

    #[test]
    fn colapsa_barras_y_puntos() {
        assert_eq!(normalize_against("", "/a//b").unwrap(), "/a/b");
        assert_eq!(normalize_against("", "/a/./b").unwrap(), "/a/b");
        assert_eq!(normalize_against("", "/a/b/").unwrap(), "/a/b");
        assert_eq!(normalize_against("", "//a///b////c//").unwrap(), "/a/b/c");
    }

    #[test]
    fn resuelve_punto_punto() {
        assert_eq!(normalize_against("", "/a/../b").unwrap(), "/b");
        assert_eq!(normalize_against("", "/a/b/../c").unwrap(), "/a/c");
        assert_eq!(normalize_against("", "/a/b/../../c").unwrap(), "/c");
        assert_eq!(normalize_against("", "/a/../../b").unwrap(), "/b");
        assert_eq!(normalize_against("", "/dev/../tmp/./x").unwrap(), "/tmp/x");
    }

    /// The old contract. Kept as a test of the ONE string that must not pick up the
    /// base, because "" is not a relative path -- it names nothing. This was folded in
    /// with `rechaza_relativas` when both were true for the same reason; they are not
    /// the same rule and only one of them survived.
    #[test]
    fn rechaza_vacias() {
        assert_eq!(normalize_against("", ""), Err(KError::InvalidArgument));
        assert_eq!(normalize_against("/tmp", ""), Err(KError::InvalidArgument));
        assert_eq!(normalize_against("/tmp/x", ""), Err(KError::InvalidArgument));
    }

    #[test]
    fn resuelve_relativas_contra_la_base() {
        assert_eq!(normalize_against("/tmp", "x").unwrap(), "/tmp/x");
        assert_eq!(normalize_against("/tmp", "./x").unwrap(), "/tmp/x");
        assert_eq!(normalize_against("/tmp", "a/b").unwrap(), "/tmp/a/b");
        assert_eq!(normalize_against("/", "a/b").unwrap(), "/a/b");
        // `.` and `..` naming the cwd and its parent -- the thing that makes `rm .`
        // expressible at the VFS boundary at last, instead of arriving pre-collapsed.
        assert_eq!(normalize_against("/tmp", ".").unwrap(), "/tmp");
        assert_eq!(normalize_against("/tmp/x", "..").unwrap(), "/tmp");
        // .. pops ACROSS the join, which is the whole reason the base goes in as
        // components rather than as a string prefix.
        assert_eq!(normalize_against("/tmp/x", "../y").unwrap(), "/tmp/y");
        assert_eq!(normalize_against("/a/b/c", "../../d").unwrap(), "/a/d");
        // Escaping past the root clamps, it does not underflow.
        assert_eq!(normalize_against("/", "..").unwrap(), "/");
        assert_eq!(normalize_against("/tmp", "../../..").unwrap(), "/");
    }

    #[test]
    fn una_ruta_absoluta_ignora_la_base() {
        assert_eq!(normalize_against("/tmp", "/a/b").unwrap(), "/a/b");
        assert_eq!(normalize_against("/tmp/x", "/").unwrap(), "/");
    }

    #[test]
    fn nombre_demasiado_largo() {
        let mut largo = String::from("/");
        for _ in 0..(MAX_NAME_LEN + 1) {
            largo.push('a');
        }
        assert_eq!(normalize_against("", &largo), Err(KError::NameTooLong));
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
