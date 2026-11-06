#![allow(dead_code)]

mod wire;

use crate::arch::xtensa::sync::Mutex;
use crate::drivers::flash;
use crate::prelude::*;
use crate::vfs::inode::{
    default_mode, kind_code, DirEntry, FileSystem, FsStat, Inode, InodeKind, StatMeta,
};
use alloc::collections::BTreeMap;

use wire::*;

const SEC: u32 = layout::FLASH_SECTOR_SIZE as u32;

const ROOT_INO: u32 = 1;

const MAX_NAME: usize = 255;

const COMPACT_CHUNK: usize = 2048;

#[derive(Clone, Copy)]
struct Geom {
    super_off: [u32; 2],
    half_off: [u32; 2],
    half_size: u32,
}

fn geometry() -> Geom {
    let base = layout::FS_OFFSET;
    let total_sectors = layout::FS_SIZE / SEC;
    let log_base = base + 2 * SEC;
    let half_sectors = (total_sectors - 2) / 2;
    let half_size = half_sectors * SEC;
    Geom {
        super_off: [base, base + SEC],
        half_off: [log_base, log_base + half_size],
        half_size,
    }
}

#[derive(Clone, Copy)]
struct Extent {
    off: u32,
    len: u32,
    flash: u32,
}

struct FileData {
    size: u32,
    extents: Vec<Extent>,
}

enum Body {
    Dir(BTreeMap<String, u32>),
    File(FileData),
    Symlink(String),
}

struct Node {
    kind: InodeKind,
    mode: u16,
    uid: u16,
    gid: u16,
    body: Body,
}

impl Node {
    fn size_of(&self) -> u64 {
        match &self.body {
            Body::File(f) => f.size as u64,
            Body::Symlink(s) => s.len() as u64,
            Body::Dir(_) => 0,
        }
    }
}

fn zeroed(len: usize) -> Vec<u8> {
    let mut v = Vec::new();
    v.resize(len, 0u8);
    v
}

struct FsState {
    geom: Geom,
    nodes: BTreeMap<u32, Node>,
    next_ino: u32,
    active_half: usize,
    cursor: u32,
    next_seq: u32,
    generation: u32,
    super_slot: usize,
}

impl FsState {
    fn empty(geom: Geom, active: usize, generation: u32, super_slot: usize) -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            ROOT_INO,
            Node {
                kind: InodeKind::Dir,
                mode: 0o0755,
                uid: 0,
                gid: 0,
                body: Body::Dir(BTreeMap::new()),
            },
        );
        FsState {
            geom,
            nodes,
            next_ino: 2,
            active_half: active,
            cursor: geom.half_off[active],
            next_seq: 1,
            generation,
            super_slot,
        }
    }

    fn apply_mk(&mut self, kind: InodeKind, ino: u32, parent: u32, name: &str) {
        let body = match kind {
            InodeKind::Dir => Body::Dir(BTreeMap::new()),
            _ => Body::File(FileData {
                size: 0,
                extents: Vec::new(),
            }),
        };
        self.nodes.insert(
            ino,
            Node {
                kind,
                mode: default_mode(kind),
                uid: 0,
                gid: 0,
                body,
            },
        );
        if let Some(Node {
            body: Body::Dir(m), ..
        }) = self.nodes.get_mut(&parent)
        {
            m.insert(String::from(name), ino);
        }
        if ino >= self.next_ino {
            self.next_ino = ino + 1;
        }
    }

    fn apply_symlink(&mut self, ino: u32, parent: u32, name: &str, target: &str) {
        self.nodes.insert(
            ino,
            Node {
                kind: InodeKind::Symlink,
                mode: 0o0777,
                uid: 0,
                gid: 0,
                body: Body::Symlink(String::from(target)),
            },
        );
        if let Some(Node {
            body: Body::Dir(m), ..
        }) = self.nodes.get_mut(&parent)
        {
            m.insert(String::from(name), ino);
        }
        if ino >= self.next_ino {
            self.next_ino = ino + 1;
        }
    }

    fn apply_setattr(&mut self, ino: u32, mode: u16, uid: u16, gid: u16) {
        if let Some(node) = self.nodes.get_mut(&ino) {
            node.mode = mode;
            node.uid = uid;
            node.gid = gid;
        }
    }

    fn apply_rename(&mut self, old_parent: u32, new_parent: u32, old_name: &str, new_name: &str) {
        let moved_ino = match self.nodes.get_mut(&old_parent) {
            Some(Node {
                body: Body::Dir(m), ..
            }) => m.remove(old_name),
            _ => None,
        };
        if let Some(ino) = moved_ino {
            let replaced = match self.nodes.get_mut(&new_parent) {
                Some(Node {
                    body: Body::Dir(m), ..
                }) => m.insert(String::from(new_name), ino),
                _ => None,
            };
            if let Some(old_dst_ino) = replaced {
                if old_dst_ino != ino {
                    self.nodes.remove(&old_dst_ino);
                }
            }
        }
    }

    fn apply_write_extent(&mut self, ino: u32, off: u32, len: u32, flash_off: u32) {
        if let Some(Node {
            body: Body::File(f),
            ..
        }) = self.nodes.get_mut(&ino)
        {
            splice_extent(f, off, len, flash_off);
        }
    }

    fn apply_trunc(&mut self, ino: u32, len: u32) {
        if let Some(Node {
            body: Body::File(f),
            ..
        }) = self.nodes.get_mut(&ino)
        {
            let mut out = Vec::new();
            for e in f.extents.drain(..) {
                let es = e.off;
                let ee = e.off + e.len;
                if es >= len {
                    continue;
                }
                if ee <= len {
                    out.push(e);
                } else {
                    out.push(Extent {
                        off: es,
                        len: len - es,
                        flash: e.flash,
                    });
                }
            }
            f.extents = out;
            f.size = len;
        }
    }

    fn apply_unlink(&mut self, parent: u32, name: &str) {
        let child = match self.nodes.get(&parent) {
            Some(Node {
                body: Body::Dir(m), ..
            }) => m.get(name).copied(),
            _ => None,
        };
        if let Some(ci) = child {
            if let Some(Node {
                body: Body::Dir(m), ..
            }) = self.nodes.get_mut(&parent)
            {
                m.remove(name);
            }
            self.nodes.remove(&ci);
        }
    }

    fn read_file(&self, ino: u32, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let node = self.nodes.get(&ino).ok_or(KError::NotFound)?;
        let f = match &node.body {
            Body::File(f) => f,
            Body::Dir(_) => return Err(KError::IsADirectory),
            Body::Symlink(_) => return Err(KError::InvalidArgument),
        };
        let off = match u32::try_from(off) {
            Ok(v) => v,
            Err(_) => return Ok(0),
        };
        if off >= f.size {
            return Ok(0);
        }
        let n = core::cmp::min(buf.len() as u64, (f.size - off) as u64) as usize;
        for b in buf[..n].iter_mut() {
            *b = 0;
        }
        let read_end = off + n as u32;
        for e in &f.extents {
            let es = e.off;
            let ee = e.off + e.len;
            let s = core::cmp::max(es, off);
            let en = core::cmp::min(ee, read_end);
            if s < en {
                let flash_off = e.flash + (s - es);
                let dst_s = (s - off) as usize;
                let dst_e = (en - off) as usize;
                flash::read(flash_off, &mut buf[dst_s..dst_e])?;
            }
        }
        Ok(n)
    }

    fn create(&mut self, parent: u32, name: &str, kind: InodeKind) -> KResult<u32> {
        match self.nodes.get(&parent) {
            Some(Node {
                body: Body::Dir(m), ..
            }) => {
                if m.contains_key(name) {
                    return Err(KError::AlreadyExists);
                }
            }
            Some(_) => return Err(KError::NotADirectory),
            None => return Err(KError::NotFound),
        }
        if name.is_empty() || name.len() > MAX_NAME {
            return Err(KError::NameTooLong);
        }
        let rtype = match kind {
            InodeKind::File => RecType::MkFile,
            InodeKind::Dir => RecType::MkDir,
            _ => return Err(KError::NotSupported),
        };
        let ino = self.next_ino;
        let payload = enc_mk(ino, parent, name.as_bytes());
        self.append(rtype, &payload)?;
        self.apply_mk(kind, ino, parent, name);
        let (uid, gid) = crate::scheduler::process::caller_creds();
        if uid != 0 || gid != 0 {
            let mode = default_mode(kind);
            let ap = enc_setattr(ino, mode, uid, gid);
            let _ = self.append(RecType::SetAttr, &ap);
            self.apply_setattr(ino, mode, uid, gid);
        }
        Ok(ino)
    }

    fn create_symlink(&mut self, parent: u32, name: &str, target: &str) -> KResult<u32> {
        match self.nodes.get(&parent) {
            Some(Node {
                body: Body::Dir(m), ..
            }) => {
                if m.contains_key(name) {
                    return Err(KError::AlreadyExists);
                }
            }
            Some(_) => return Err(KError::NotADirectory),
            None => return Err(KError::NotFound),
        }
        if name.is_empty() || name.len() > MAX_NAME || target.is_empty() || target.len() > MAX_NAME
        {
            return Err(KError::NameTooLong);
        }
        let ino = self.next_ino;
        let payload = enc_symlink(ino, parent, name.as_bytes(), target.as_bytes());
        self.append(RecType::MkSymlink, &payload)?;
        self.apply_symlink(ino, parent, name, target);
        Ok(ino)
    }

    fn set_mode(&mut self, ino: u32, mode: u16) -> KResult<()> {
        let (uid, gid) = match self.nodes.get(&ino) {
            Some(n) => (n.uid, n.gid),
            None => return Err(KError::NotFound),
        };
        let m = mode & 0o7777;
        let payload = enc_setattr(ino, m, uid, gid);
        self.append(RecType::SetAttr, &payload)?;
        self.apply_setattr(ino, m, uid, gid);
        Ok(())
    }

    fn set_owner(&mut self, ino: u32, uid: Option<u16>, gid: Option<u16>) -> KResult<()> {
        let (mode, cur_u, cur_g) = match self.nodes.get(&ino) {
            Some(n) => (n.mode, n.uid, n.gid),
            None => return Err(KError::NotFound),
        };
        let nu = uid.unwrap_or(cur_u);
        let ng = gid.unwrap_or(cur_g);
        let payload = enc_setattr(ino, mode, nu, ng);
        self.append(RecType::SetAttr, &payload)?;
        self.apply_setattr(ino, mode, nu, ng);
        Ok(())
    }

    fn readlink(&self, ino: u32) -> KResult<String> {
        let node = self.nodes.get(&ino).ok_or(KError::NotFound)?;
        match &node.body {
            Body::Symlink(t) => Ok(t.clone()),
            _ => Err(KError::InvalidArgument),
        }
    }

    fn walk_parent(&self, rel: &str) -> KResult<(u32, String)> {
        let comps: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        if comps.is_empty() {
            return Err(KError::InvalidArgument);
        }
        let mut cur = ROOT_INO;
        for &c in &comps[..comps.len() - 1] {
            cur = self.lookup(cur, c)?;
        }
        Ok((cur, String::from(comps[comps.len() - 1])))
    }

    fn rename_rel(&mut self, old_rel: &str, new_rel: &str) -> KResult<()> {
        if old_rel == new_rel {
            return Ok(());
        }
        let (old_parent, old_name) = self.walk_parent(old_rel)?;
        let (new_parent, new_name) = self.walk_parent(new_rel)?;
        if new_name.is_empty() || new_name.len() > MAX_NAME {
            return Err(KError::NameTooLong);
        }
        let _src_ino = self.lookup(old_parent, &old_name)?;
        if let Ok(dst_ino) = self.lookup(new_parent, &new_name) {
            if let Some(Node {
                body: Body::Dir(sub),
                ..
            }) = self.nodes.get(&dst_ino)
            {
                if !sub.is_empty() {
                    return Err(KError::Busy);
                }
            }
        }
        let payload = enc_rename(
            old_parent,
            new_parent,
            old_name.as_bytes(),
            new_name.as_bytes(),
        );
        self.append(RecType::Rename, &payload)?;
        self.apply_rename(old_parent, new_parent, &old_name, &new_name);
        Ok(())
    }

    fn write_file(&mut self, ino: u32, off: u64, buf: &[u8]) -> KResult<usize> {
        match self.nodes.get(&ino) {
            Some(Node {
                body: Body::File(_),
                ..
            }) => {}
            Some(_) => return Err(KError::IsADirectory),
            None => return Err(KError::NotFound),
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let off = u32::try_from(off).map_err(|_| KError::InvalidArgument)?;
        let end = off.checked_add(buf.len() as u32).ok_or(KError::NoSpace)?;
        let _ = end;
        let payload = enc_write(ino, off, buf);
        let rec_off = self.append(RecType::Write, &payload)?;
        let data_flash = rec_off + HEADER_LEN as u32 + WRITE_DATA_OFF as u32;
        self.apply_write_extent(ino, off, buf.len() as u32, data_flash);
        Ok(buf.len())
    }

    fn truncate_file(&mut self, ino: u32, len: u64) -> KResult<()> {
        match self.nodes.get(&ino) {
            Some(Node {
                body: Body::File(_),
                ..
            }) => {}
            Some(_) => return Err(KError::IsADirectory),
            None => return Err(KError::NotFound),
        }
        let len = u32::try_from(len).map_err(|_| KError::InvalidArgument)?;
        let payload = enc_trunc(ino, len);
        self.append(RecType::Truncate, &payload)?;
        self.apply_trunc(ino, len);
        Ok(())
    }

    fn unlink(&mut self, parent: u32, name: &str) -> KResult<()> {
        let (dir_mode, dir_uid, child) = match self.nodes.get(&parent) {
            Some(Node {
                mode,
                uid,
                body: Body::Dir(m),
                ..
            }) => (*mode, *uid, m.get(name).copied()),
            Some(_) => return Err(KError::NotADirectory),
            None => return Err(KError::NotFound),
        };
        let ci = child.ok_or(KError::NotFound)?;

        let child_uid = match self.nodes.get(&ci) {
            Some(Node {
                uid,
                body: Body::Dir(sub),
                ..
            }) => {
                if !sub.is_empty() {
                    return Err(KError::Busy);
                }
                *uid
            }
            Some(n) => n.uid,
            None => 0,
        };

        let (caller_uid, _) = crate::scheduler::process::caller_creds();
        if (dir_mode & 0o1000) != 0
            && caller_uid != 0
            && caller_uid != dir_uid
            && caller_uid != child_uid
        {
            return Err(KError::PermissionDenied);
        }

        let payload = enc_unlink(parent, name.as_bytes());
        self.append(RecType::Unlink, &payload)?;
        self.apply_unlink(parent, name);
        Ok(())
    }

    fn readdir(&self, ino: u32, index: usize) -> KResult<Option<DirEntry>> {
        let node = self.nodes.get(&ino).ok_or(KError::NotFound)?;
        match &node.body {
            Body::Dir(m) => match m.iter().nth(index) {
                Some((name, &cino)) => {
                    let kind = self
                        .nodes
                        .get(&cino)
                        .map(|n| n.kind)
                        .unwrap_or(InodeKind::File);
                    Ok(Some(DirEntry {
                        name: name.clone(),
                        kind,
                        ino: cino as u64,
                    }))
                }
                None => Ok(None),
            },
            Body::File(_) | Body::Symlink(_) => Err(KError::NotADirectory),
        }
    }

    fn lookup(&self, ino: u32, name: &str) -> KResult<u32> {
        let node = self.nodes.get(&ino).ok_or(KError::NotFound)?;
        match &node.body {
            Body::Dir(m) => m.get(name).copied().ok_or(KError::NotFound),
            Body::File(_) | Body::Symlink(_) => Err(KError::NotADirectory),
        }
    }

    fn half_end(&self, half: usize) -> u32 {
        self.geom.half_off[half] + self.geom.half_size
    }

    fn append(&mut self, rtype: RecType, payload: &[u8]) -> KResult<u32> {
        let need = record_total_len(payload.len()) as u32;
        if self.cursor + need > self.half_end(self.active_half) {
            self.compact()?;
            if self.cursor + need > self.half_end(self.active_half) {
                return Err(KError::NoSpace);
            }
        }
        let rec = encode_record(rtype, self.next_seq, payload);
        let off = self.cursor;
        flash::write(off, &rec)?;
        self.cursor += rec.len() as u32;
        self.next_seq += 1;
        Ok(off)
    }

    fn compact(&mut self) -> KResult<()> {
        // Compaction is rare -- it fires only when the active half fills -- and it is
        // the heaviest, most intricate thing this filesystem does: it erases a half,
        // replays the whole live tree into it, and rewrites every extent. A silent hang
        // that only reproduced after many boots was first suspected here, precisely
        // because it runs so seldom. Log it: an operator seeing this line right before a
        // failure knows where to look, and its absence rules the path out.
        //
        // Sparse Hole Preservation: To avoid expanding sparse holes into explicit
        // zero-filled flash records during compaction, all-zero chunks are detected
        // and skipped. If trailing sparse holes exist (or the file is completely sparse),
        // a lightweight Truncate record is appended to preserve the logical file size
        // without consuming flash data payload space.
        crate::println!("[espfs] compacting half {} -> {}", self.active_half, 1 - self.active_half);
        let dst = 1 - self.active_half;
        let dbase = self.geom.half_off[dst];
        let dend = dbase + self.geom.half_size;

        let mut o = dbase;
        while o < dend {
            flash::erase_sector(o)?;
            o += SEC;
        }

        let plan = self.build_plan();

        let mut cur = dbase;
        let mut seq = 0u32;
        let mut new_ext: BTreeMap<u32, Vec<Extent>> = BTreeMap::new();

        for (ino, parent, name, kind) in &plan {
            let rec = if *kind == InodeKind::Symlink {
                let target = match self.nodes.get(ino) {
                    Some(Node {
                        body: Body::Symlink(t),
                        ..
                    }) => t.as_bytes(),
                    _ => b"",
                };
                let payload = enc_symlink(*ino, *parent, name.as_bytes(), target);
                encode_record(RecType::MkSymlink, seq, &payload)
            } else {
                let rtype = if *kind == InodeKind::Dir {
                    RecType::MkDir
                } else {
                    RecType::MkFile
                };
                let payload = enc_mk(*ino, *parent, name.as_bytes());
                encode_record(rtype, seq, &payload)
            };
            if cur + rec.len() as u32 > dend {
                return Err(KError::NoSpace);
            }
            flash::write(cur, &rec)?;
            cur += rec.len() as u32;
            seq += 1;

            if let Some(n) = self.nodes.get(ino) {
                if n.mode != default_mode(*kind) || n.uid != 0 || n.gid != 0 {
                    let ap = enc_setattr(*ino, n.mode, n.uid, n.gid);
                    let arec = encode_record(RecType::SetAttr, seq, &ap);
                    if cur + arec.len() as u32 > dend {
                        return Err(KError::NoSpace);
                    }
                    flash::write(cur, &arec)?;
                    cur += arec.len() as u32;
                    seq += 1;
                }
            }

            if *kind == InodeKind::File {
                let size = self.nodes.get(ino).map(|n| n.size_of() as u32).unwrap_or(0);
                let mut exts = Vec::new();
                let mut coff = 0u32;
                while coff < size {
                    let clen = core::cmp::min(COMPACT_CHUNK as u32, size - coff);
                    let mut tmp = zeroed(clen as usize);

                    self.read_file(*ino, coff as u64, &mut tmp)?;
                    if tmp.iter().all(|&b| b == 0) {
                        // Skip writing all-zero chunk to preserve sparse holes
                        coff += clen;
                        continue;
                    }
                    let payload = enc_write(*ino, coff, &tmp);
                    let rec = encode_record(RecType::Write, seq, &payload);
                    if cur + rec.len() as u32 > dend {
                        return Err(KError::NoSpace);
                    }
                    let data_flash = cur + HEADER_LEN as u32 + WRITE_DATA_OFF as u32;
                    flash::write(cur, &rec)?;
                    exts.push(Extent {
                        off: coff,
                        len: clen,
                        flash: data_flash,
                    });
                    cur += rec.len() as u32;
                    seq += 1;
                    coff += clen;
                }
                let last_end = exts.last().map(|e| e.off + e.len).unwrap_or(0);
                if last_end < size {
                    let payload = enc_trunc(*ino, size);
                    let rec = encode_record(RecType::Truncate, seq, &payload);
                    if cur + rec.len() as u32 > dend {
                        return Err(KError::NoSpace);
                    }
                    flash::write(cur, &rec)?;
                    cur += rec.len() as u32;
                    seq += 1;
                }
                new_ext.insert(*ino, exts);
            }
        }

        // Commit, then apply -- in that order, and the order is the point.
        //
        // The new half is fully written at this point, but nothing durable yet says so:
        // the on-flash superblock still names the old half. write_super is the commit.
        // It is also the one step here that does flash I/O and can therefore fail, and
        // its `?` bails out of compact entirely.
        //
        // So everything that mutates in-memory state -- the scalar swap AND the new
        // extents -- has to happen AFTER it. This loop used to run first: on a
        // write_super error it left every file's extents pointing into the new half
        // while active_half/cursor and the flash superblock still described the old one,
        // an inconsistency that only a reboot cleared. Now an early Err leaves both
        // memory and flash describing the old half, unchanged, which is exactly what a
        // half-done compaction should look like: as if it never started.
        let new_slot = 1 - self.super_slot;
        let new_gen = self.generation + 1;
        self.write_super(new_slot, new_gen, dst as u32)?;

        self.active_half = dst;
        self.cursor = cur;
        self.next_seq = seq;
        self.generation = new_gen;
        self.super_slot = new_slot;

        for (ino, exts) in new_ext {
            if let Some(Node {
                body: Body::File(f),
                ..
            }) = self.nodes.get_mut(&ino)
            {
                f.extents = exts;
            }
        }
        Ok(())
    }

    fn build_plan(&self) -> Vec<(u32, u32, String, InodeKind)> {
        let mut plan = Vec::new();
        let mut stack: Vec<u32> = alloc::vec![ROOT_INO];
        while let Some(dir) = stack.pop() {
            if let Some(Node {
                body: Body::Dir(m), ..
            }) = self.nodes.get(&dir)
            {
                for (name, &cino) in m.iter() {
                    let kind = self
                        .nodes
                        .get(&cino)
                        .map(|n| n.kind)
                        .unwrap_or(InodeKind::File);
                    plan.push((cino, dir, name.clone(), kind));
                    if kind == InodeKind::Dir {
                        stack.push(cino);
                    }
                }
            }
        }
        plan
    }

    fn write_super(&self, slot: usize, generation: u32, active_half: u32) -> KResult<()> {
        let off = self.geom.super_off[slot];
        flash::erase_sector(off)?;
        let enc = encode_super(SuperBlock {
            generation,
            active_half,
        });
        flash::write(off, &enc)?;
        Ok(())
    }

    fn replay(&mut self, half: usize) -> KResult<()> {
        let base = self.geom.half_off[half];
        let end = base + self.geom.half_size;
        let mut cur = base;
        let mut hbuf = [0u8; HEADER_LEN];
        loop {
            if cur + HEADER_LEN as u32 > end {
                break;
            }
            if flash::read(cur, &mut hbuf).is_err() {
                break;
            }
            let h = match parse_header(&hbuf) {
                Some(h) => h,
                None => break,
            };
            let plen = h.plen as usize;
            let total = record_total_len(plen) as u32;
            if cur.checked_add(total).map(|e| e > end).unwrap_or(true) {
                break;
            }
            let payload_off = cur + HEADER_LEN as u32;

            let mut crc = crc32_update(crc32_init(), &hbuf[0..12]);
            let mut remaining = plen;
            let mut o = payload_off;
            let mut chunk = [0u8; 256];
            let mut crc_ok = true;
            while remaining > 0 {
                let c = core::cmp::min(remaining, chunk.len());
                if flash::read(o, &mut chunk[..c]).is_err() {
                    crc_ok = false;
                    break;
                }
                crc = crc32_update(crc, &chunk[..c]);
                o += c as u32;
                remaining -= c;
            }
            if !crc_ok || crc32_final(crc) != h.crc {
                break;
            }

            match h.rtype {
                RecType::Write => {
                    let mut p8 = [0u8; 8];
                    if flash::read(payload_off, &mut p8).is_err() {
                        break;
                    }
                    if let Some((ino, off)) = dec_write_head(&p8) {
                        let dlen = plen.saturating_sub(WRITE_DATA_OFF) as u32;
                        let dflash = payload_off + WRITE_DATA_OFF as u32;
                        self.apply_write_extent(ino, off, dlen, dflash);
                    }
                }
                RecType::MkFile | RecType::MkDir => {
                    let mut p = zeroed(plen);
                    if flash::read(payload_off, &mut p).is_err() {
                        break;
                    }
                    if let Some((ino, parent, name)) = dec_mk(&p) {
                        if let Ok(name) = core::str::from_utf8(name) {
                            let kind = if h.rtype == RecType::MkDir {
                                InodeKind::Dir
                            } else {
                                InodeKind::File
                            };
                            self.apply_mk(kind, ino, parent, name);
                        }
                    }
                }
                RecType::Truncate => {
                    let mut p = [0u8; 8];
                    if flash::read(payload_off, &mut p).is_err() {
                        break;
                    }
                    if let Some((ino, len)) = dec_trunc(&p) {
                        self.apply_trunc(ino, len);
                    }
                }
                RecType::Unlink => {
                    let mut p = zeroed(plen);
                    if flash::read(payload_off, &mut p).is_err() {
                        break;
                    }
                    if let Some((parent, name)) = dec_unlink(&p) {
                        if let Ok(name) = core::str::from_utf8(name) {
                            self.apply_unlink(parent, name);
                        }
                    }
                }
                RecType::MkSymlink => {
                    let mut p = zeroed(plen);
                    if flash::read(payload_off, &mut p).is_err() {
                        break;
                    }
                    if let Some((ino, parent, name, target)) = dec_symlink(&p) {
                        if let (Ok(name), Ok(target)) =
                            (core::str::from_utf8(name), core::str::from_utf8(target))
                        {
                            self.apply_symlink(ino, parent, name, target);
                        }
                    }
                }
                RecType::SetAttr => {
                    let mut p = zeroed(plen);
                    if flash::read(payload_off, &mut p).is_err() {
                        break;
                    }
                    if let Some((ino, mode, uid, gid)) = dec_setattr(&p) {
                        self.apply_setattr(ino, mode, uid, gid);
                    }
                }
                RecType::Rename => {
                    let mut p = zeroed(plen);
                    if flash::read(payload_off, &mut p).is_err() {
                        break;
                    }
                    if let Some((old_p, new_p, old_n, new_n)) = dec_rename(&p) {
                        if let (Ok(old_s), Ok(new_s)) =
                            (core::str::from_utf8(old_n), core::str::from_utf8(new_n))
                        {
                            self.apply_rename(old_p, new_p, old_s, new_s);
                        }
                    }
                }
            }
            if h.seq >= self.next_seq {
                self.next_seq = h.seq + 1;
            }
            cur += total;
        }
        self.cursor = cur;
        Ok(())
    }

    fn load(geom: Geom) -> KResult<Option<FsState>> {
        let mut a = [0u8; SB_LEN];
        let mut b = [0u8; SB_LEN];
        flash::read(geom.super_off[0], &mut a)?;
        flash::read(geom.super_off[1], &mut b)?;
        let sa = decode_super(&a);
        let sb = decode_super(&b);
        let (chosen, slot) = match (sa, sb) {
            (Some(x), Some(y)) => {
                if y.generation > x.generation {
                    (y, 1)
                } else {
                    (x, 0)
                }
            }
            (Some(x), None) => (x, 0),
            (None, Some(y)) => (y, 1),
            (None, None) => return Ok(None),
        };
        let active = if chosen.active_half >= 2 {
            0
        } else {
            chosen.active_half as usize
        };
        let mut st = FsState::empty(geom, active, chosen.generation, slot);
        st.replay(active)?;
        Ok(Some(st))
    }

    fn format(geom: Geom) -> KResult<FsState> {
        flash::erase_sector(geom.super_off[0])?;
        flash::erase_sector(geom.super_off[1])?;
        let mut o = geom.half_off[0];
        let end = o + geom.half_size;
        while o < end {
            flash::erase_sector(o)?;
            o += SEC;
        }
        let enc = encode_super(SuperBlock {
            generation: 1,
            active_half: 0,
        });
        flash::write(geom.super_off[0], &enc)?;
        Ok(FsState::empty(geom, 0, 1, 0))
    }
}

fn splice_extent(f: &mut FileData, off: u32, len: u32, flash_off: u32) {
    if len == 0 {
        return;
    }
    let new_end = off + len;
    let mut out: Vec<Extent> = Vec::new();
    for e in f.extents.drain(..) {
        let es = e.off;
        let ee = e.off + e.len;
        if ee <= off || es >= new_end {
            out.push(e);
        } else {
            if es < off {
                out.push(Extent {
                    off: es,
                    len: off - es,
                    flash: e.flash,
                });
            }
            if ee > new_end {
                out.push(Extent {
                    off: new_end,
                    len: ee - new_end,
                    flash: e.flash + (new_end - es),
                });
            }
        }
    }
    out.push(Extent {
        off,
        len,
        flash: flash_off,
    });
    out.sort_by_key(|e| e.off);
    f.extents = out;
    if new_end > f.size {
        f.size = new_end;
    }
}

struct EspFsInner {
    state: Mutex<FsState>,
}

impl EspFsInner {
    fn kind(&self, ino: u32) -> InodeKind {
        self.state
            .lock()
            .nodes
            .get(&ino)
            .map(|n| n.kind)
            .unwrap_or(InodeKind::File)
    }
    fn size(&self, ino: u32) -> u64 {
        self.state
            .lock()
            .nodes
            .get(&ino)
            .map(|n| n.size_of())
            .unwrap_or(0)
    }
    fn meta(&self, ino: u32) -> StatMeta {
        let st = self.state.lock();
        if let Some(n) = st.nodes.get(&ino) {
            StatMeta {
                ino: ino as u64,
                size: n.size_of(),
                mode: n.mode,
                kind: kind_code(n.kind),
                nlink: if n.kind == InodeKind::Dir { 2 } else { 1 },
                uid: n.uid,
                gid: n.gid,
                mtime: 0,
            }
        } else {
            StatMeta {
                ino: ino as u64,
                size: 0,
                mode: 0o0644,
                kind: 1,
                nlink: 1,
                uid: 0,
                gid: 0,
                mtime: 0,
            }
        }
    }
    fn readlink(&self, ino: u32) -> KResult<String> {
        self.state.lock().readlink(ino)
    }
    fn create_symlink(&self, parent: u32, name: &str, target: &str) -> KResult<u32> {
        self.state.lock().create_symlink(parent, name, target)
    }
    fn set_mode(&self, ino: u32, mode: u16) -> KResult<()> {
        self.state.lock().set_mode(ino, mode)
    }
    fn set_owner(&self, ino: u32, uid: Option<u16>, gid: Option<u16>) -> KResult<()> {
        self.state.lock().set_owner(ino, uid, gid)
    }
    fn rename_rel(&self, old_rel: &str, new_rel: &str) -> KResult<()> {
        self.state.lock().rename_rel(old_rel, new_rel)
    }
    fn read_at(&self, ino: u32, off: u64, buf: &mut [u8]) -> KResult<usize> {
        self.state.lock().read_file(ino, off, buf)
    }
    fn write_at(&self, ino: u32, off: u64, buf: &[u8]) -> KResult<usize> {
        self.state.lock().write_file(ino, off, buf)
    }
    fn truncate(&self, ino: u32, len: u64) -> KResult<()> {
        self.state.lock().truncate_file(ino, len)
    }
    fn readdir(&self, ino: u32, index: usize) -> KResult<Option<DirEntry>> {
        self.state.lock().readdir(ino, index)
    }
    fn lookup(&self, ino: u32, name: &str) -> KResult<u32> {
        self.state.lock().lookup(ino, name)
    }
    fn create(&self, parent: u32, name: &str, kind: InodeKind) -> KResult<u32> {
        self.state.lock().create(parent, name, kind)
    }
    fn unlink(&self, parent: u32, name: &str) -> KResult<()> {
        self.state.lock().unlink(parent, name)
    }
    fn used_bytes(&self) -> u64 {
        let st = self.state.lock();
        (st.cursor - st.geom.half_off[st.active_half]) as u64
    }
    fn total_bytes(&self) -> u64 {
        self.state.lock().geom.half_size as u64
    }
}

struct EspNode {
    inner: Arc<EspFsInner>,
    ino: u32,
}

impl EspNode {
    fn make(inner: Arc<EspFsInner>, ino: u32) -> Arc<dyn Inode> {
        Arc::new(EspNode { inner, ino })
    }
}

impl Inode for EspNode {
    fn kind(&self) -> InodeKind {
        self.inner.kind(self.ino)
    }
    fn inode_key(&self) -> (usize, u64) {
        (Arc::as_ptr(&self.inner) as usize, self.ino as u64)
    }
    fn size(&self) -> u64 {
        self.inner.size(self.ino)
    }
    fn meta(&self) -> StatMeta {
        self.inner.meta(self.ino)
    }
    fn readlink(&self) -> KResult<String> {
        self.inner.readlink(self.ino)
    }
    fn create_symlink(&self, name: &str, target: &str) -> KResult<Arc<dyn Inode>> {
        let cino = self.inner.create_symlink(self.ino, name, target)?;
        Ok(EspNode::make(self.inner.clone(), cino))
    }
    fn set_mode(&self, mode: u16) -> KResult<()> {
        self.inner.set_mode(self.ino, mode)
    }
    fn set_owner(&self, uid: Option<u16>, gid: Option<u16>) -> KResult<()> {
        self.inner.set_owner(self.ino, uid, gid)
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        self.inner.read_at(self.ino, off, buf)
    }
    fn write_at(&self, off: u64, buf: &[u8]) -> KResult<usize> {
        self.inner.write_at(self.ino, off, buf)
    }
    fn truncate(&self, len: u64) -> KResult<()> {
        self.inner.truncate(self.ino, len)
    }
    fn readdir(&self, index: usize) -> KResult<Option<DirEntry>> {
        self.inner.readdir(self.ino, index)
    }
    fn lookup(&self, name: &str) -> KResult<Arc<dyn Inode>> {
        let cino = self.inner.lookup(self.ino, name)?;
        Ok(EspNode::make(self.inner.clone(), cino))
    }
    fn create(&self, name: &str, kind: InodeKind) -> KResult<Arc<dyn Inode>> {
        let cino = self.inner.create(self.ino, name, kind)?;
        Ok(EspNode::make(self.inner.clone(), cino))
    }
    fn unlink(&self, name: &str) -> KResult<()> {
        self.inner.unlink(self.ino, name)
    }
    fn sync(&self) -> KResult<()> {
        Ok(())
    }
}

pub struct EspFs {
    inner: Arc<EspFsInner>,
}

impl EspFs {
    pub fn mount() -> KResult<Arc<EspFs>> {
        let geom = geometry();
        let st = match FsState::load(geom)? {
            Some(s) => s,
            None => FsState::format(geom)?,
        };
        Ok(Arc::new(EspFs {
            inner: Arc::new(EspFsInner {
                state: Mutex::new(st),
            }),
        }))
    }

    pub fn format_and_mount() -> KResult<Arc<EspFs>> {
        let geom = geometry();
        let st = FsState::format(geom)?;
        Ok(Arc::new(EspFs {
            inner: Arc::new(EspFsInner {
                state: Mutex::new(st),
            }),
        }))
    }
}

impl FileSystem for EspFs {
    fn name(&self) -> &str {
        "espfs"
    }
    fn root(&self) -> Arc<dyn Inode> {
        EspNode::make(self.inner.clone(), ROOT_INO)
    }
    fn sync(&self) -> KResult<()> {
        Ok(())
    }
    fn stat(&self) -> FsStat {
        FsStat {
            total_bytes: self.inner.total_bytes(),
            used_bytes: self.inner.used_bytes(),
            block_size: SEC,
        }
    }
    fn rename(&self, old_rel: &str, new_rel: &str) -> KResult<()> {
        self.inner.rename_rel(old_rel, new_rel)
    }
}
