#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::drivers::flash;
use crate::prelude::*;
use crate::vfs::inode::{
    default_mode, kind_code, DirEntry, FileSystem, FsStat, Inode, InodeKind, StatMeta,
};
use alloc::collections::BTreeMap;

const LFS_MAGIC: u32 = 0x5346_4C45; // "ELFS"
const LFS_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug)]
pub struct LfsConfig {
    pub region_offset: u32,
    pub region_len: u32,
    pub read_size: u32,
    pub prog_size: u32,
    pub block_size: u32,
    pub block_count: u32,
    pub block_cycles: i32,
    pub cache_size: u32,
    pub lookahead_size: u32,
}

impl LfsConfig {
    pub const fn for_region(offset: u32, len: u32) -> LfsConfig {
        let block_size = flash::SECTOR_SIZE as u32;
        LfsConfig {
            region_offset: offset,
            region_len: len,
            read_size: 16,
            prog_size: 16,
            block_size,
            block_count: len / block_size,
            block_cycles: 500,
            cache_size: 256,
            lookahead_size: 32,
        }
    }

    pub fn abs_offset(&self, block: u32, off: u32, size: u32) -> KResult<u32> {
        if block >= self.block_count {
            return Err(KError::InvalidArgument);
        }
        match off.checked_add(size) {
            Some(fin) if fin <= self.block_size => {}
            _ => return Err(KError::InvalidArgument),
        }
        let block_byte = block
            .checked_mul(self.block_size)
            .ok_or(KError::InvalidArgument)?;
        let within = block_byte.checked_add(off).ok_or(KError::InvalidArgument)?;
        self.region_offset
            .checked_add(within)
            .ok_or(KError::InvalidArgument)
    }

    pub fn dev_read(&self, block: u32, off: u32, buf: &mut [u8]) -> KResult<()> {
        let abs = self.abs_offset(block, off, buf.len() as u32)?;
        flash::read(abs, buf)
    }

    pub fn dev_prog(&self, block: u32, off: u32, buf: &[u8]) -> KResult<()> {
        let abs = self.abs_offset(block, off, buf.len() as u32)?;
        flash::write(abs, buf)
    }

    pub fn dev_erase(&self, block: u32) -> KResult<()> {
        let abs = self.abs_offset(block, 0, self.block_size)?;
        flash::erase_sector(abs)
    }

    pub fn dev_sync(&self) -> KResult<()> {
        Ok(())
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

enum LfsBody {
    Dir(BTreeMap<String, u32>),
    File(Vec<u8>),
    Symlink(String),
}

struct LfsNodeData {
    kind: InodeKind,
    mode: u16,
    uid: u16,
    gid: u16,
    body: LfsBody,
}

struct LfsState {
    config: LfsConfig,
    nodes: BTreeMap<u32, LfsNodeData>,
    next_ino: u32,
}

impl LfsState {
    fn empty(config: LfsConfig) -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            1,
            LfsNodeData {
                kind: InodeKind::Dir,
                mode: 0o0755,
                uid: 0,
                gid: 0,
                body: LfsBody::Dir(BTreeMap::new()),
            },
        );
        Self {
            config,
            nodes,
            next_ino: 2,
        }
    }

    fn serialize(&self) -> Vec<u8> {
        let mut payload = Vec::new();
        let mut stack = alloc::vec![1u32];
        while let Some(dir_ino) = stack.pop() {
            if let Some(LfsNodeData {
                body: LfsBody::Dir(children),
                ..
            }) = self.nodes.get(&dir_ino)
            {
                for (name, &cino) in children.iter() {
                    if let Some(cnode) = self.nodes.get(&cino) {
                        let k = kind_code(cnode.kind);
                        let nbytes = name.as_bytes();
                        let dbytes: &[u8] = match &cnode.body {
                            LfsBody::File(v) => v.as_slice(),
                            LfsBody::Symlink(s) => s.as_bytes(),
                            LfsBody::Dir(_) => &[],
                        };
                        payload.push(k);
                        payload.extend_from_slice(&cino.to_le_bytes());
                        payload.extend_from_slice(&dir_ino.to_le_bytes());
                        payload.extend_from_slice(&cnode.mode.to_le_bytes());
                        payload.extend_from_slice(&cnode.uid.to_le_bytes());
                        payload.extend_from_slice(&cnode.gid.to_le_bytes());
                        payload.extend_from_slice(&(nbytes.len() as u16).to_le_bytes());
                        payload.extend_from_slice(&(dbytes.len() as u32).to_le_bytes());
                        payload.extend_from_slice(nbytes);
                        payload.extend_from_slice(dbytes);
                        if cnode.kind == InodeKind::Dir {
                            stack.push(cino);
                        }
                    }
                }
            }
        }
        payload
    }

    fn flush_to_flash(&self) -> KResult<()> {
        let payload = self.serialize();
        let mut image = Vec::with_capacity(16 + payload.len() + 4);
        image.extend_from_slice(&LFS_MAGIC.to_le_bytes());
        image.extend_from_slice(&LFS_VERSION.to_le_bytes());
        image.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        image.extend_from_slice(&crc32(&payload).to_le_bytes());
        image.extend_from_slice(&payload);
        while image.len() % 4 != 0 {
            image.push(0);
        }
        if image.len() as u32 > self.config.region_len {
            return Err(KError::NoSpace);
        }

        let bsz = self.config.block_size as usize;
        let blocks_needed = (image.len() + bsz - 1) / bsz;
        for b in 0..blocks_needed as u32 {
            self.config.dev_erase(b)?;
            let start = (b as usize) * bsz;
            let end = core::cmp::min(start + bsz, image.len());
            self.config.dev_prog(b, 0, &image[start..end])?;
        }
        Ok(())
    }

    fn load_from_flash(config: LfsConfig) -> KResult<Option<Self>> {
        if config.block_count == 0 {
            return Ok(None);
        }
        let mut hdr = [0u8; 16];
        config.dev_read(0, 0, &mut hdr)?;
        let magic = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let ver = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
        let plen = u32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]) as usize;
        let expected_crc = u32::from_le_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]);
        if magic != LFS_MAGIC || ver != LFS_VERSION {
            return Ok(None);
        }
        if (16 + plen) as u32 > config.region_len {
            return Ok(None);
        }
        let mut payload = Vec::new();
        payload.resize(plen, 0u8);
        let bsz = config.block_size as usize;
        let mut copied = 0usize;
        let mut abs_pos = 16usize;
        while copied < plen {
            let blk = (abs_pos / bsz) as u32;
            let boff = (abs_pos % bsz) as u32;
            let chunk = core::cmp::min(plen - copied, bsz - (boff as usize));
            config.dev_read(blk, boff, &mut payload[copied..copied + chunk])?;
            copied += chunk;
            abs_pos += chunk;
        }
        if crc32(&payload) != expected_crc {
            return Ok(None);
        }

        let mut st = Self::empty(config);
        let mut pos = 0usize;
        while pos + 21 <= payload.len() {
            let k = payload[pos];
            let ino = u32::from_le_bytes([
                payload[pos + 1],
                payload[pos + 2],
                payload[pos + 3],
                payload[pos + 4],
            ]);
            let parent = u32::from_le_bytes([
                payload[pos + 5],
                payload[pos + 6],
                payload[pos + 7],
                payload[pos + 8],
            ]);
            let mode = u16::from_le_bytes([payload[pos + 9], payload[pos + 10]]);
            let uid = u16::from_le_bytes([payload[pos + 11], payload[pos + 12]]);
            let gid = u16::from_le_bytes([payload[pos + 13], payload[pos + 14]]);
            let nlen = u16::from_le_bytes([payload[pos + 15], payload[pos + 16]]) as usize;
            let dlen = u32::from_le_bytes([
                payload[pos + 17],
                payload[pos + 18],
                payload[pos + 19],
                payload[pos + 20],
            ]) as usize;
            pos += 21;
            if pos + nlen + dlen > payload.len() {
                break;
            }
            let name = match core::str::from_utf8(&payload[pos..pos + nlen]) {
                Ok(s) => String::from(s),
                Err(_) => break,
            };
            pos += nlen;
            let data = &payload[pos..pos + dlen];
            pos += dlen;

            let (kind, body) = match k {
                2 => (InodeKind::Dir, LfsBody::Dir(BTreeMap::new())),
                4 => (
                    InodeKind::Symlink,
                    LfsBody::Symlink(String::from_utf8_lossy(data).into_owned()),
                ),
                _ => (InodeKind::File, LfsBody::File(data.to_vec())),
            };
            st.nodes.insert(
                ino,
                LfsNodeData {
                    kind,
                    mode,
                    uid,
                    gid,
                    body,
                },
            );
            if let Some(LfsNodeData {
                body: LfsBody::Dir(m),
                ..
            }) = st.nodes.get_mut(&parent)
            {
                m.insert(name, ino);
            }
            if ino >= st.next_ino {
                st.next_ino = ino + 1;
            }
        }
        Ok(Some(st))
    }

    fn used_bytes(&self) -> u64 {
        let mut sum = 16u64;
        for n in self.nodes.values() {
            match &n.body {
                LfsBody::File(v) => sum += v.len() as u64 + 24,
                LfsBody::Symlink(s) => sum += s.len() as u64 + 24,
                LfsBody::Dir(_) => sum += 24,
            }
        }
        sum
    }
}

struct LfsInner {
    state: Mutex<LfsState>,
}

struct LfsInode {
    inner: Arc<LfsInner>,
    ino: u32,
}

impl LfsInode {
    fn make(inner: Arc<LfsInner>, ino: u32) -> Arc<dyn Inode> {
        Arc::new(Self { inner, ino })
    }
}

impl Inode for LfsInode {
    fn kind(&self) -> InodeKind {
        self.inner
            .state
            .lock()
            .nodes
            .get(&self.ino)
            .map(|n| n.kind)
            .unwrap_or(InodeKind::File)
    }

    fn inode_key(&self) -> (usize, u64) {
        (Arc::as_ptr(&self.inner) as usize, self.ino as u64)
    }

    fn size(&self) -> u64 {
        let st = self.inner.state.lock();
        match st.nodes.get(&self.ino).map(|n| &n.body) {
            Some(LfsBody::File(v)) => v.len() as u64,
            Some(LfsBody::Symlink(s)) => s.len() as u64,
            _ => 0,
        }
    }

    fn meta(&self) -> StatMeta {
        let st = self.inner.state.lock();
        if let Some(n) = st.nodes.get(&self.ino) {
            let sz = match &n.body {
                LfsBody::File(v) => v.len() as u64,
                LfsBody::Symlink(s) => s.len() as u64,
                LfsBody::Dir(_) => 0,
            };
            StatMeta {
                ino: self.ino as u64,
                size: sz,
                mode: n.mode,
                kind: kind_code(n.kind),
                nlink: if n.kind == InodeKind::Dir { 2 } else { 1 },
                uid: n.uid,
                gid: n.gid,
                mtime: 0,
            }
        } else {
            StatMeta {
                ino: self.ino as u64,
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

    fn readlink(&self) -> KResult<String> {
        let st = self.inner.state.lock();
        match st.nodes.get(&self.ino).map(|n| &n.body) {
            Some(LfsBody::Symlink(t)) => Ok(t.clone()),
            Some(_) => Err(KError::InvalidArgument),
            None => Err(KError::NotFound),
        }
    }

    fn create_symlink(&self, name: &str, target: &str) -> KResult<Arc<dyn Inode>> {
        let mut st = self.inner.state.lock();
        match st.nodes.get(&self.ino) {
            Some(LfsNodeData {
                body: LfsBody::Dir(m),
                ..
            }) => {
                if m.contains_key(name) {
                    return Err(KError::AlreadyExists);
                }
            }
            Some(_) => return Err(KError::NotADirectory),
            None => return Err(KError::NotFound),
        }
        let cino = st.next_ino;
        st.next_ino += 1;
        st.nodes.insert(
            cino,
            LfsNodeData {
                kind: InodeKind::Symlink,
                mode: 0o0777,
                uid: 0,
                gid: 0,
                body: LfsBody::Symlink(String::from(target)),
            },
        );
        if let Some(LfsNodeData {
            body: LfsBody::Dir(m),
            ..
        }) = st.nodes.get_mut(&self.ino)
        {
            m.insert(String::from(name), cino);
        }
        st.flush_to_flash()?;
        Ok(LfsInode::make(self.inner.clone(), cino))
    }

    fn set_mode(&self, mode: u16) -> KResult<()> {
        let mut st = self.inner.state.lock();
        let n = st.nodes.get_mut(&self.ino).ok_or(KError::NotFound)?;
        n.mode = mode & 0o7777;
        st.flush_to_flash()
    }

    fn set_owner(&self, uid: Option<u16>, gid: Option<u16>) -> KResult<()> {
        let mut st = self.inner.state.lock();
        let n = st.nodes.get_mut(&self.ino).ok_or(KError::NotFound)?;
        if let Some(u) = uid {
            n.uid = u;
        }
        if let Some(g) = gid {
            n.gid = g;
        }
        st.flush_to_flash()
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let st = self.inner.state.lock();
        let n = st.nodes.get(&self.ino).ok_or(KError::NotFound)?;
        match &n.body {
            LfsBody::File(data) => {
                let off = off as usize;
                if off >= data.len() {
                    return Ok(0);
                }
                let cnt = core::cmp::min(buf.len(), data.len() - off);
                buf[..cnt].copy_from_slice(&data[off..off + cnt]);
                Ok(cnt)
            }
            LfsBody::Dir(_) => Err(KError::IsADirectory),
            LfsBody::Symlink(_) => Err(KError::InvalidArgument),
        }
    }

    fn write_at(&self, off: u64, buf: &[u8]) -> KResult<usize> {
        let mut st = self.inner.state.lock();
        {
            let n = st.nodes.get_mut(&self.ino).ok_or(KError::NotFound)?;
            match &mut n.body {
                LfsBody::File(data) => {
                    let off = off as usize;
                    let end = off.checked_add(buf.len()).ok_or(KError::NoMem)?;
                    if end > data.len() {
                        data.resize(end, 0);
                    }
                    data[off..end].copy_from_slice(buf);
                }
                LfsBody::Dir(_) => return Err(KError::IsADirectory),
                LfsBody::Symlink(_) => return Err(KError::InvalidArgument),
            }
        }
        st.flush_to_flash()?;
        Ok(buf.len())
    }

    fn truncate(&self, len: u64) -> KResult<()> {
        let mut st = self.inner.state.lock();
        {
            let n = st.nodes.get_mut(&self.ino).ok_or(KError::NotFound)?;
            match &mut n.body {
                LfsBody::File(data) => data.resize(len as usize, 0),
                LfsBody::Dir(_) => return Err(KError::IsADirectory),
                LfsBody::Symlink(_) => return Err(KError::InvalidArgument),
            }
        }
        st.flush_to_flash()
    }

    fn readdir(&self, index: usize) -> KResult<Option<DirEntry>> {
        let st = self.inner.state.lock();
        let n = st.nodes.get(&self.ino).ok_or(KError::NotFound)?;
        match &n.body {
            LfsBody::Dir(m) => match m.iter().nth(index) {
                Some((name, &cino)) => {
                    let kind = st.nodes.get(&cino).map(|c| c.kind).unwrap_or(InodeKind::File);
                    Ok(Some(DirEntry {
                        name: name.clone(),
                        kind,
                        ino: cino as u64,
                    }))
                }
                None => Ok(None),
            },
            _ => Err(KError::NotADirectory),
        }
    }

    fn lookup(&self, name: &str) -> KResult<Arc<dyn Inode>> {
        let st = self.inner.state.lock();
        let n = st.nodes.get(&self.ino).ok_or(KError::NotFound)?;
        match &n.body {
            LfsBody::Dir(m) => {
                let cino = m.get(name).copied().ok_or(KError::NotFound)?;
                Ok(LfsInode::make(self.inner.clone(), cino))
            }
            _ => Err(KError::NotADirectory),
        }
    }

    fn create(&self, name: &str, kind: InodeKind) -> KResult<Arc<dyn Inode>> {
        let (uid, gid) = crate::scheduler::process::caller_creds();
        let mut st = self.inner.state.lock();
        match st.nodes.get(&self.ino) {
            Some(LfsNodeData {
                body: LfsBody::Dir(m),
                ..
            }) => {
                if m.contains_key(name) {
                    return Err(KError::AlreadyExists);
                }
            }
            Some(_) => return Err(KError::NotADirectory),
            None => return Err(KError::NotFound),
        }
        let body = match kind {
            InodeKind::File => LfsBody::File(Vec::new()),
            InodeKind::Dir => LfsBody::Dir(BTreeMap::new()),
            _ => return Err(KError::NotSupported),
        };
        let cino = st.next_ino;
        st.next_ino += 1;
        st.nodes.insert(
            cino,
            LfsNodeData {
                kind,
                mode: default_mode(kind),
                uid,
                gid,
                body,
            },
        );
        if let Some(LfsNodeData {
            body: LfsBody::Dir(m),
            ..
        }) = st.nodes.get_mut(&self.ino)
        {
            m.insert(String::from(name), cino);
        }
        st.flush_to_flash()?;
        Ok(LfsInode::make(self.inner.clone(), cino))
    }

    fn unlink(&self, name: &str) -> KResult<()> {
        let mut st = self.inner.state.lock();
        let cino = match st.nodes.get(&self.ino) {
            Some(LfsNodeData {
                body: LfsBody::Dir(m),
                ..
            }) => m.get(name).copied().ok_or(KError::NotFound)?,
            Some(_) => return Err(KError::NotADirectory),
            None => return Err(KError::NotFound),
        };
        if let Some(LfsNodeData {
            body: LfsBody::Dir(sub),
            ..
        }) = st.nodes.get(&cino)
        {
            if !sub.is_empty() {
                return Err(KError::Busy);
            }
        }
        if let Some(LfsNodeData {
            body: LfsBody::Dir(m),
            ..
        }) = st.nodes.get_mut(&self.ino)
        {
            m.remove(name);
        }
        st.nodes.remove(&cino);
        st.flush_to_flash()
    }

    fn sync(&self) -> KResult<()> {
        self.inner.state.lock().flush_to_flash()
    }
}

pub struct LittleFs {
    inner: Arc<LfsInner>,
}

impl LittleFs {
    pub fn mount(offset: u32, len: u32, format_if_empty: bool) -> KResult<Arc<LittleFs>> {
        Self::validar_region(offset, len)?;
        let config = LfsConfig::for_region(offset, len);
        let st = match LfsState::load_from_flash(config)? {
            Some(loaded) => loaded,
            None => {
                let empty = LfsState::empty(config);
                if format_if_empty {
                    empty.flush_to_flash()?;
                }
                empty
            }
        };
        Ok(Arc::new(LittleFs {
            inner: Arc::new(LfsInner {
                state: Mutex::new(st),
            }),
        }))
    }

    pub fn mount_default() -> KResult<Arc<LittleFs>> {
        Self::mount(layout::AUX_LFS_OFFSET, layout::AUX_LFS_SIZE, true)
    }

    pub fn format(offset: u32, len: u32) -> KResult<()> {
        Self::validar_region(offset, len)?;
        let config = LfsConfig::for_region(offset, len);
        let empty = LfsState::empty(config);
        empty.flush_to_flash()
    }

    fn validar_region(offset: u32, len: u32) -> KResult<()> {
        let sector = flash::SECTOR_SIZE as u32;
        if len == 0 || sector == 0 {
            return Err(KError::InvalidArgument);
        }
        if offset % sector != 0 || len % sector != 0 {
            return Err(KError::InvalidArgument);
        }
        let fin = offset.checked_add(len).ok_or(KError::InvalidArgument)?;
        if fin > layout::FLASH_SIZE {
            return Err(KError::InvalidArgument);
        }
        Ok(())
    }
}

impl FileSystem for LittleFs {
    fn name(&self) -> &str {
        "littlefs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        LfsInode::make(self.inner.clone(), 1)
    }

    fn sync(&self) -> KResult<()> {
        self.inner.state.lock().flush_to_flash()
    }

    fn stat(&self) -> FsStat {
        let st = self.inner.state.lock();
        FsStat {
            total_bytes: st.config.region_len as u64,
            used_bytes: st.used_bytes(),
            block_size: st.config.block_size,
        }
    }
}
