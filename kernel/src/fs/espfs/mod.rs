#![allow(dead_code)]

//! EspFs — sistema de archivos persistente *log-structured* sobre el flash NOR
//! interno (Fase 4). Implementa los traits `vfs::inode::{FileSystem, Inode}`, por
//! lo que se monta igual que `ramfs` pero sobrevive a reinicios.
//!
//! ## Diseño
//! La región `fs` del flash ([`layout::FS_OFFSET`], [`layout::FS_SIZE`]) se divide en:
//! - 2 sectores de **superbloque** (ping-pong para tolerancia a cortes).
//! - 2 **mitades** de log de igual tamaño (A y B).
//!
//! El estado vive en RAM como un árbol de inodos; en flash sólo hay una secuencia
//! append-only de registros (ver [`wire`]). Al montar se reproduce el log de la
//! mitad activa. Cada operación de escritura añade un registro (durable al
//! instante). Cuando la mitad activa se llena, se **compacta**: se reescribe el
//! conjunto vivo en la otra mitad y se conmuta el superbloque; un corte durante la
//! compactación deja intacta la generación anterior.
//!
//! ## Límites conocidos
//! - Las escrituras a flash bloquean interrupciones (vía esp-storage). Hacerlas con
//!   el WiFi activo (p. ej. `write` sobre SSH) puede perturbar la radio; validar en
//!   hardware. La ruta por consola (UART) es segura.
//! - La compactación es una sección crítica larga: pausa el planificador mientras
//!   dura (rara: sólo tras ~4 MB de escrituras).

mod wire;

use crate::arch::xtensa::sync::Mutex;
use crate::drivers::flash;
use crate::prelude::*;
use crate::vfs::inode::{DirEntry, FileSystem, FsStat, Inode, InodeKind};
use alloc::collections::BTreeMap;

use wire::*;

/// Tamaño de sector del flash NOR.
const SEC: u32 = layout::FLASH_SECTOR_SIZE as u32;

/// Inodo raíz (siempre presente, implícito, no se registra en el log).
const ROOT_INO: u32 = 1;

/// Longitud máxima de nombre.
const MAX_NAME: usize = 255;

/// Tamaño de trozo al reescribir contenido durante la compactación.
const COMPACT_CHUNK: usize = 2048;

// ===========================================================================
// Geometría.
// ===========================================================================

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

// ===========================================================================
// Índice en RAM.
// ===========================================================================

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
}

struct Node {
    kind: InodeKind,
    body: Body,
}

impl Node {
    fn size_of(&self) -> u64 {
        match &self.body {
            Body::File(f) => f.size as u64,
            Body::Dir(_) => 0,
        }
    }
}

fn zeroed(len: usize) -> Vec<u8> {
    let mut v = Vec::new();
    v.resize(len, 0u8);
    v
}

// ===========================================================================
// Estado del sistema de archivos (protegido por Mutex).
// ===========================================================================

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

    // --- Aplicación de registros (RAM), compartida por replay y ops en vivo ---

    fn apply_mk(&mut self, kind: InodeKind, ino: u32, parent: u32, name: &str) {
        let body = match kind {
            InodeKind::Dir => Body::Dir(BTreeMap::new()),
            _ => Body::File(FileData {
                size: 0,
                extents: Vec::new(),
            }),
        };
        self.nodes.insert(ino, Node { kind, body });
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

    // --- Lectura de contenido ---

    fn read_file(&self, ino: u32, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let node = self.nodes.get(&ino).ok_or(KError::NotFound)?;
        let f = match &node.body {
            Body::File(f) => f,
            Body::Dir(_) => return Err(KError::IsADirectory),
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

    // --- Operaciones en vivo (añaden registro y actualizan RAM) ---

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
        Ok(ino)
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
        let child = match self.nodes.get(&parent) {
            Some(Node {
                body: Body::Dir(m), ..
            }) => m.get(name).copied(),
            Some(_) => return Err(KError::NotADirectory),
            None => return Err(KError::NotFound),
        };
        let ci = child.ok_or(KError::NotFound)?;
        // No borrar directorios no vacíos.
        if let Some(Node {
            body: Body::Dir(sub),
            ..
        }) = self.nodes.get(&ci)
        {
            if !sub.is_empty() {
                return Err(KError::Busy);
            }
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
            Body::File(_) => Err(KError::NotADirectory),
        }
    }

    fn lookup(&self, ino: u32, name: &str) -> KResult<u32> {
        let node = self.nodes.get(&ino).ok_or(KError::NotFound)?;
        match &node.body {
            Body::Dir(m) => m.get(name).copied().ok_or(KError::NotFound),
            Body::File(_) => Err(KError::NotADirectory),
        }
    }

    // --- Append + compactación ---

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

    /// Reescribe el conjunto vivo en la otra mitad y conmuta el superbloque.
    fn compact(&mut self) -> KResult<()> {
        let dst = 1 - self.active_half;
        let dbase = self.geom.half_off[dst];
        let dend = dbase + self.geom.half_size;

        // Borrar la mitad destino.
        let mut o = dbase;
        while o < dend {
            flash::erase_sector(o)?;
            o += SEC;
        }

        // Recorrido en preorden desde la raíz: (ino, parent, name, kind).
        let plan = self.build_plan();

        let mut cur = dbase;
        let mut seq = 0u32;
        let mut new_ext: BTreeMap<u32, Vec<Extent>> = BTreeMap::new();

        for (ino, parent, name, kind) in &plan {
            // Registro de creación.
            let rtype = if *kind == InodeKind::Dir {
                RecType::MkDir
            } else {
                RecType::MkFile
            };
            let payload = enc_mk(*ino, *parent, name.as_bytes());
            let rec = encode_record(rtype, seq, &payload);
            if cur + rec.len() as u32 > dend {
                return Err(KError::NoSpace);
            }
            flash::write(cur, &rec)?;
            cur += rec.len() as u32;
            seq += 1;

            // Contenido de archivos (reescrito en trozos, coalescido).
            if *kind == InodeKind::File {
                let size = self.nodes.get(ino).map(|n| n.size_of() as u32).unwrap_or(0);
                let mut exts = Vec::new();
                let mut coff = 0u32;
                while coff < size {
                    let clen = core::cmp::min(COMPACT_CHUNK as u32, size - coff);
                    let mut tmp = zeroed(clen as usize);
                    // Leer desde las extensiones actuales (mitad origen intacta).
                    self.read_file(*ino, coff as u64, &mut tmp)?;
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
                new_ext.insert(*ino, exts);
            }
        }

        // Aplicar las nuevas extensiones al índice.
        for (ino, exts) in new_ext {
            if let Some(Node {
                body: Body::File(f),
                ..
            }) = self.nodes.get_mut(&ino)
            {
                f.extents = exts;
            }
        }

        // Nuevo superbloque (durabilidad: se escribe DESPUÉS del log nuevo).
        let new_slot = 1 - self.super_slot;
        let new_gen = self.generation + 1;
        self.write_super(new_slot, new_gen, dst as u32)?;

        self.active_half = dst;
        self.cursor = cur;
        self.next_seq = seq;
        self.generation = new_gen;
        self.super_slot = new_slot;
        Ok(())
    }

    /// Lista (ino, parent, name, kind) en preorden desde la raíz (excluye la raíz).
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

    /// Reproduce el log de la mitad `half`, dejando `cursor` al final válido.
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
            // Verificar CRC leyendo el payload en trozos (memoria acotada).
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
                break; // cola rota = fin del log
            }
            // Aplicar.
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
            }
            if h.seq >= self.next_seq {
                self.next_seq = h.seq + 1;
            }
            cur += total;
        }
        self.cursor = cur;
        Ok(())
    }

    /// Intenta cargar un FS existente. `Ok(None)` = sin superbloque válido (no formateado).
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

    /// Formatea la región y devuelve un FS vacío montado.
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

/// Inserta/solapa una extensión manteniendo la lista ordenada y sin solapes.
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

// ===========================================================================
// Envoltura compartida + inodos.
// ===========================================================================

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
    fn size(&self) -> u64 {
        self.inner.size(self.ino)
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
        // Cada escritura ya es durable (append al log). Nada que vaciar.
        Ok(())
    }
}

pub struct EspFs {
    inner: Arc<EspFsInner>,
}

impl EspFs {
    /// Monta el FS existente; si la región no está formateada, la formatea.
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

    /// Formatea la región y monta un FS vacío.
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
}
