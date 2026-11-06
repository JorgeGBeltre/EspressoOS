#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;
use crate::vfs::inode::{
    default_mode, kind_code, DirEntry, FileSystem, FsStat, Inode, InodeKind, StatMeta,
};
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU32, Ordering};

static NEXT_INO: AtomicU32 = AtomicU32::new(1);

fn alloc_ino() -> u64 {
    NEXT_INO.fetch_add(1, Ordering::Relaxed) as u64
}

fn now_sec() -> u64 {
    crate::arch::xtensa::timer::uptime_ms() / 1000
}

enum RamBody {
    File(Vec<u8>),
    Dir(BTreeMap<String, Arc<RamNode>>),
    Fifo(Arc<crate::vfs::pipe::Pipe>),
    Symlink(String),
}

#[derive(Clone, Copy)]
struct RamAttr {
    mode: u16,
    uid: u16,
    gid: u16,
    mtime: u64,
}

struct RamNode {
    ino: u64,
    kind: InodeKind,
    attr: Mutex<RamAttr>,
    body: Mutex<RamBody>,
}

impl RamNode {
    fn new_with(kind: InodeKind, body: RamBody) -> Arc<RamNode> {
        let (uid, gid) = crate::scheduler::process::caller_creds();
        Arc::new(RamNode {
            ino: alloc_ino(),
            kind,
            attr: Mutex::new(RamAttr {
                mode: default_mode(kind),
                uid,
                gid,
                mtime: now_sec(),
            }),
            body: Mutex::new(body),
        })
    }

    fn new_dir() -> Arc<RamNode> {
        Self::new_with(InodeKind::Dir, RamBody::Dir(BTreeMap::new()))
    }

    fn new_file() -> Arc<RamNode> {
        Self::new_with(InodeKind::File, RamBody::File(Vec::new()))
    }

    fn new_fifo() -> Arc<RamNode> {
        Self::new_with(
            InodeKind::Fifo,
            RamBody::Fifo(crate::vfs::pipe::Pipe::new(4096)),
        )
    }

    fn new_symlink(target: &str) -> Arc<RamNode> {
        Self::new_with(InodeKind::Symlink, RamBody::Symlink(String::from(target)))
    }
}

impl Inode for RamNode {
    fn kind(&self) -> InodeKind {
        self.kind
    }

    fn inode_key(&self) -> (usize, u64) {
        (self as *const Self as usize, self.ino)
    }

    fn size(&self) -> u64 {
        let body = self.body.lock();
        match &*body {
            RamBody::File(data) => data.len() as u64,
            RamBody::Symlink(target) => target.len() as u64,
            RamBody::Dir(_) | RamBody::Fifo(_) => 0,
        }
    }

    fn meta(&self) -> StatMeta {
        let size = self.size();
        let attr = *self.attr.lock();
        StatMeta {
            ino: self.ino,
            size,
            mode: attr.mode,
            kind: kind_code(self.kind),
            nlink: if self.kind == InodeKind::Dir { 2 } else { 1 },
            uid: attr.uid,
            gid: attr.gid,
            mtime: attr.mtime,
        }
    }

    fn readlink(&self) -> KResult<String> {
        let body = self.body.lock();
        match &*body {
            RamBody::Symlink(target) => Ok(target.clone()),
            _ => Err(KError::InvalidArgument),
        }
    }

    fn create_symlink(&self, name: &str, target: &str) -> KResult<Arc<dyn Inode>> {
        let mut body = self.body.lock();
        match &mut *body {
            RamBody::Dir(children) => {
                if children.contains_key(name) {
                    return Err(KError::AlreadyExists);
                }
                let node = RamNode::new_symlink(target);
                children.insert(String::from(name), node.clone());
                let out: Arc<dyn Inode> = node;
                Ok(out)
            }
            _ => Err(KError::NotADirectory),
        }
    }

    fn set_mode(&self, mode: u16) -> KResult<()> {
        let mut attr = self.attr.lock();
        attr.mode = mode & 0o7777;
        attr.mtime = now_sec();
        Ok(())
    }

    fn set_owner(&self, uid: Option<u16>, gid: Option<u16>) -> KResult<()> {
        let mut attr = self.attr.lock();
        if let Some(u) = uid {
            attr.uid = u;
        }
        if let Some(g) = gid {
            attr.gid = g;
        }
        attr.mtime = now_sec();
        Ok(())
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let pipe_opt = {
            let body = self.body.lock();
            match &*body {
                RamBody::File(data) => {
                    let start = match usize::try_from(off) {
                        Ok(v) => v,
                        Err(_) => return Ok(0),
                    };
                    if start >= data.len() {
                        return Ok(0);
                    }
                    let disponible = data.len() - start;
                    let n = core::cmp::min(disponible, buf.len());
                    buf[..n].copy_from_slice(&data[start..start + n]);
                    return Ok(n);
                }
                RamBody::Dir(_) => return Err(KError::IsADirectory),
                RamBody::Symlink(_) => return Err(KError::InvalidArgument),
                RamBody::Fifo(p) => p.clone(),
            }
        };

        let reader = crate::vfs::pipe::PipeReadInode::new(pipe_opt);
        reader.read_at(off, buf)
    }

    fn write_at(&self, off: u64, buf: &[u8]) -> KResult<usize> {
        let pipe_opt = {
            let mut body = self.body.lock();
            match &mut *body {
                RamBody::File(data) => {
                    let start = usize::try_from(off).map_err(|_| KError::InvalidArgument)?;
                    let end = start.checked_add(buf.len()).ok_or(KError::NoMem)?;
                    if end > data.len() {
                        let extra = end - data.len();
                        data.try_reserve(extra).map_err(|_| KError::NoMem)?;
                        data.resize(end, 0);
                    }
                    data[start..end].copy_from_slice(buf);
                    drop(body);
                    self.attr.lock().mtime = now_sec();
                    return Ok(buf.len());
                }
                RamBody::Dir(_) => return Err(KError::IsADirectory),
                RamBody::Symlink(_) => return Err(KError::InvalidArgument),
                RamBody::Fifo(p) => p.clone(),
            }
        };

        let writer = crate::vfs::pipe::PipeWriteInode::new(pipe_opt);
        writer.write_at(off, buf)
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
                drop(body);
                self.attr.lock().mtime = now_sec();
                Ok(())
            }
            RamBody::Dir(_) => Err(KError::IsADirectory),
            RamBody::Symlink(_) => Err(KError::InvalidArgument),
            RamBody::Fifo(_) => Ok(()),
        }
    }

    fn readdir(&self, index: usize) -> KResult<Option<DirEntry>> {
        let body = self.body.lock();
        match &*body {
            RamBody::Dir(children) => match children.iter().nth(index) {
                Some((name, node)) => Ok(Some(DirEntry {
                    name: name.clone(),
                    kind: node.kind,
                    ino: node.ino,
                })),
                None => Ok(None),
            },
            _ => Err(KError::NotADirectory),
        }
    }

    fn lookup(&self, name: &str) -> KResult<Arc<dyn Inode>> {
        let body = self.body.lock();
        match &*body {
            RamBody::Dir(children) => match children.get(name) {
                Some(node) => {
                    let out: Arc<dyn Inode> = node.clone();
                    Ok(out)
                }
                None => Err(KError::NotFound),
            },
            _ => Err(KError::NotADirectory),
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
                    InodeKind::Fifo => RamNode::new_fifo(),
                    InodeKind::Device | InodeKind::Symlink => return Err(KError::NotSupported),
                };
                children.insert(String::from(name), node.clone());
                let out: Arc<dyn Inode> = node;
                Ok(out)
            }
            _ => Err(KError::NotADirectory),
        }
    }

    fn unlink(&self, name: &str) -> KResult<()> {
        let (caller_uid, _) = crate::scheduler::process::caller_creds();
        let dir_attr = *self.attr.lock();

        let mut body = self.body.lock();
        match &mut *body {
            RamBody::Dir(children) => {
                let (vacio, child_uid) = match children.get(name) {
                    None => return Err(KError::NotFound),
                    Some(node) => {
                        let cuid = node.attr.lock().uid;
                        let child_body = node.body.lock();
                        let empty = match &*child_body {
                            RamBody::Dir(sub) => sub.is_empty(),
                            RamBody::File(_) | RamBody::Fifo(_) | RamBody::Symlink(_) => true,
                        };
                        (empty, cuid)
                    }
                };
                if !vacio {
                    return Err(KError::Busy);
                }
                if (dir_attr.mode & 0o1000) != 0
                    && caller_uid != 0
                    && caller_uid != dir_attr.uid
                    && caller_uid != child_uid
                {
                    return Err(KError::PermissionDenied);
                }
                children.remove(name);
                Ok(())
            }
            _ => Err(KError::NotADirectory),
        }
    }
}

pub struct RamFs {
    root: Arc<RamNode>,
}

impl RamFs {
    pub fn new() -> Arc<RamFs> {
        Arc::new(RamFs {
            root: RamNode::new_dir(),
        })
    }

    fn used_bytes(node: &Arc<RamNode>) -> u64 {
        let body = node.body.lock();
        match &*body {
            RamBody::File(data) => data.len() as u64,
            RamBody::Symlink(t) => t.len() as u64,
            RamBody::Dir(children) => {
                let mut total: u64 = 0;
                for (_, child) in children.iter() {
                    total = total.saturating_add(Self::used_bytes(child));
                }
                total
            }
            RamBody::Fifo(_) => 0,
        }
    }

    fn walk_parent(&self, rel: &str) -> KResult<(Arc<RamNode>, String)> {
        let comps: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        if comps.is_empty() {
            return Err(KError::InvalidArgument);
        }
        let mut cur = self.root.clone();
        for &c in &comps[..comps.len() - 1] {
            let next = {
                let b = cur.body.lock();
                match &*b {
                    RamBody::Dir(m) => m.get(c).cloned().ok_or(KError::NotFound)?,
                    _ => return Err(KError::NotADirectory),
                }
            };
            cur = next;
        }
        Ok((cur, String::from(comps[comps.len() - 1])))
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
        Ok(())
    }

    fn stat(&self) -> FsStat {
        let usado = RamFs::used_bytes(&self.root);
        let libre = crate::mm::heap::stats().free as u64;
        FsStat {
            total_bytes: libre.saturating_add(usado),
            used_bytes: usado,
            block_size: 1,
        }
    }

    fn rename(&self, old_rel: &str, new_rel: &str) -> KResult<()> {
        if old_rel == new_rel {
            return Ok(());
        }
        let (old_parent, old_name) = self.walk_parent(old_rel)?;
        let (new_parent, new_name) = self.walk_parent(new_rel)?;

        if Arc::ptr_eq(&old_parent, &new_parent) {
            let mut body = old_parent.body.lock();
            let children = match &mut *body {
                RamBody::Dir(m) => m,
                _ => return Err(KError::NotADirectory),
            };
            let moved = children.get(&old_name).cloned().ok_or(KError::NotFound)?;
            if let Some(existing) = children.get(&new_name) {
                let eb = existing.body.lock();
                if let RamBody::Dir(sub) = &*eb {
                    if !sub.is_empty() {
                        return Err(KError::Busy);
                    }
                }
            }
            children.remove(&old_name);
            children.insert(new_name, moved);
            return Ok(());
        }

        let moved = {
            let body = old_parent.body.lock();
            match &*body {
                RamBody::Dir(m) => m.get(&old_name).cloned().ok_or(KError::NotFound)?,
                _ => return Err(KError::NotADirectory),
            }
        };

        {
            let mut dst_body = new_parent.body.lock();
            let dst_children = match &mut *dst_body {
                RamBody::Dir(m) => m,
                _ => return Err(KError::NotADirectory),
            };
            if let Some(existing) = dst_children.get(&new_name) {
                let eb = existing.body.lock();
                if let RamBody::Dir(sub) = &*eb {
                    if !sub.is_empty() {
                        return Err(KError::Busy);
                    }
                }
            }
            dst_children.insert(new_name, moved);
        }

        let mut src_body = old_parent.body.lock();
        if let RamBody::Dir(m) = &mut *src_body {
            m.remove(&old_name);
        }
        Ok(())
    }
}
