#![allow(dead_code)]

use crate::drivers::flash;
use crate::prelude::*;

pub const SLOT_COUNT: u32 = 2;

pub const OTA_SELECT_ENTRY_SIZE: usize = 32;

pub const OTA_SEQ_EMPTY: u32 = 0xFFFF_FFFF;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum OtaImgState {
    New = 0x0,

    PendingVerify = 0x1,

    Valid = 0x2,

    Invalid = 0x3,

    Aborted = 0x4,

    Undefined = 0xFFFF_FFFF,
}

impl OtaImgState {
    pub const fn from_raw(v: u32) -> OtaImgState {
        match v {
            0x0 => OtaImgState::New,
            0x1 => OtaImgState::PendingVerify,
            0x2 => OtaImgState::Valid,
            0x3 => OtaImgState::Invalid,
            0x4 => OtaImgState::Aborted,
            _ => OtaImgState::Undefined,
        }
    }

    pub const fn as_raw(self) -> u32 {
        self as u32
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Factory,

    Ota0,
}

impl Slot {
    pub const fn region(self) -> (u32, u32) {
        match self {
            Slot::Factory => (layout::FACTORY_OFFSET, layout::FACTORY_SIZE),
            Slot::Ota0 => (layout::OTA0_OFFSET, layout::OTA0_SIZE),
        }
    }

    pub const fn index(self) -> u32 {
        match self {
            Slot::Factory => 0,
            Slot::Ota0 => 1,
        }
    }

    pub const fn from_index(idx: u32) -> Slot {
        match idx {
            0 => Slot::Factory,
            _ => Slot::Ota0,
        }
    }

    pub const fn other(self) -> Slot {
        match self {
            Slot::Factory => Slot::Ota0,
            Slot::Ota0 => Slot::Factory,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OtaSelectEntry {
    pub ota_seq: u32,

    pub seq_label: [u8; 20],

    pub ota_state: u32,

    pub crc: u32,
}

impl OtaSelectEntry {
    pub const fn empty() -> Self {
        Self {
            ota_seq: OTA_SEQ_EMPTY,
            seq_label: [0xFF; 20],
            ota_state: OTA_SEQ_EMPTY,
            crc: OTA_SEQ_EMPTY,
        }
    }

    pub fn new(ota_seq: u32, state: OtaImgState) -> Self {
        let mut e = Self {
            ota_seq,
            seq_label: [0x00; 20],
            ota_state: state.as_raw(),
            crc: 0,
        };
        e.crc = e.compute_crc();
        e
    }

    pub fn compute_crc(&self) -> u32 {
        crc32_le(0xFFFF_FFFF, &self.ota_seq.to_le_bytes())
    }

    pub fn is_valid(&self) -> bool {
        self.ota_seq != OTA_SEQ_EMPTY
            && self.ota_seq != 0
            && self.crc == self.compute_crc()
            && self.ota_state != OtaImgState::Invalid.as_raw()
            && self.ota_state != OtaImgState::Aborted.as_raw()
    }

    pub fn to_bytes(&self) -> [u8; OTA_SELECT_ENTRY_SIZE] {
        let mut b = [0u8; OTA_SELECT_ENTRY_SIZE];

        b[0..4].copy_from_slice(&self.ota_seq.to_le_bytes());
        b[4..24].copy_from_slice(&self.seq_label);
        b[24..28].copy_from_slice(&self.ota_state.to_le_bytes());
        b[28..32].copy_from_slice(&self.crc.to_le_bytes());
        b
    }

    pub fn from_bytes(buf: &[u8]) -> KResult<Self> {
        if buf.len() < OTA_SELECT_ENTRY_SIZE {
            return Err(KError::InvalidArgument);
        }

        let ota_seq = u32_le(&buf[0..4])?;
        let mut seq_label = [0u8; 20];
        seq_label.copy_from_slice(&buf[4..24]);
        let ota_state = u32_le(&buf[24..28])?;
        let crc = u32_le(&buf[28..32])?;
        Ok(Self {
            ota_seq,
            seq_label,
            ota_state,
            crc,
        })
    }
}

pub fn select_active_index(entries: &[OtaSelectEntry; 2]) -> Option<usize> {
    let v0 = entries[0].is_valid();
    let v1 = entries[1].is_valid();
    match (v0, v1) {
        (true, true) => {
            if entries[0].ota_seq > entries[1].ota_seq {
                Some(0)
            } else {
                Some(1)
            }
        }
        (true, false) => Some(0),
        (false, true) => Some(1),
        (false, false) => None,
    }
}

pub const fn slot_from_seq(seq: u32) -> Slot {
    Slot::from_index(seq.wrapping_sub(1) % SLOT_COUNT)
}

pub fn next_seq_for_index(current_max: u32, target_index: u32) -> KResult<u32> {
    if target_index >= SLOT_COUNT {
        return Err(KError::InvalidArgument);
    }

    let base = target_index.checked_add(1).ok_or(KError::InvalidArgument)?;
    let mut seq = current_max.checked_add(1).ok_or(KError::NoSpace)?;
    if seq < base {
        seq = base;
    }
    for _ in 0..SLOT_COUNT {
        if seq.wrapping_sub(1) % SLOT_COUNT == target_index {
            return Ok(seq);
        }
        seq = seq.checked_add(1).ok_or(KError::NoSpace)?;
    }

    Err(KError::InvalidArgument)
}

pub fn crc32_le(seed: u32, data: &[u8]) -> u32 {
    let mut crc = !seed;
    for &byte in data {
        crc ^= byte as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            bit += 1;
        }
    }
    !crc
}

fn u32_le(b: &[u8]) -> KResult<u32> {
    let arr: [u8; 4] = b
        .get(0..4)
        .ok_or(KError::InvalidArgument)?
        .try_into()
        .map_err(|_| KError::InvalidArgument)?;
    Ok(u32::from_le_bytes(arr))
}

fn otadata_copy_offset(copy: usize) -> KResult<u32> {
    let sector = flash::SECTOR_SIZE as u32;
    match copy {
        0 => Ok(layout::OTADATA_OFFSET),
        1 => layout::OTADATA_OFFSET
            .checked_add(sector)
            .ok_or(KError::InvalidArgument),
        _ => Err(KError::InvalidArgument),
    }
}

fn read_otadata() -> KResult<[OtaSelectEntry; 2]> {
    let mut entries = [OtaSelectEntry::empty(); 2];
    for (i, slot) in entries.iter_mut().enumerate() {
        let off = otadata_copy_offset(i)?;
        let mut raw = [0u8; OTA_SELECT_ENTRY_SIZE];
        flash::read(off, &mut raw)?;
        *slot = OtaSelectEntry::from_bytes(&raw)?;
    }
    Ok(entries)
}

fn write_otadata_copy(copy: usize, entry: &OtaSelectEntry) -> KResult<()> {
    let off = otadata_copy_offset(copy)?;

    flash::erase_sector(off)?;
    flash::write(off, &entry.to_bytes())
}

pub fn otadata_entries() -> KResult<[OtaSelectEntry; 2]> {
    read_otadata()
}

pub fn active_slot() -> Slot {
    let entries = match read_otadata() {
        Ok(e) => e,
        Err(_) => return Slot::Factory,
    };
    match select_active_index(&entries) {
        Some(i) => entries
            .get(i)
            .map(|e| slot_from_seq(e.ota_seq))
            .unwrap_or(Slot::Factory),
        None => Slot::Factory,
    }
}

pub fn set_boot_slot(slot: Slot) -> KResult<()> {
    let entries = read_otadata().unwrap_or([OtaSelectEntry::empty(); 2]);
    let active = select_active_index(&entries);

    let current_max = match active {
        Some(i) => entries.get(i).map(|e| e.ota_seq).unwrap_or(0),
        None => 0,
    };

    let target_index = slot.index();
    let new_seq = next_seq_for_index(current_max, target_index)?;

    let write_copy = match active {
        Some(0) => 1usize,
        Some(_) => 0usize,
        None => 0usize,
    };

    let entry = OtaSelectEntry::new(new_seq, OtaImgState::New);
    write_otadata_copy(write_copy, &entry)
}

pub fn get_state() -> KResult<OtaImgState> {
    let entries = read_otadata()?;
    if let Some(idx) = select_active_index(&entries) {
        Ok(OtaImgState::from_raw(entries[idx].ota_state))
    } else {
        Err(KError::NotFound)
    }
}

pub fn set_state(state: OtaImgState) -> KResult<()> {
    let entries = read_otadata()?;
    if let Some(idx) = select_active_index(&entries) {
        let mut entry = entries[idx];
        entry.ota_state = state.as_raw();
        entry.crc = entry.compute_crc();
        write_otadata_copy(idx, &entry)
    } else {
        Err(KError::NotFound)
    }
}
