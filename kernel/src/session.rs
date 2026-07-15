#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::drivers::uart;
use crate::prelude::*;
use crate::vfs::inode::{Inode, InodeKind};
use alloc::collections::{BTreeMap, VecDeque};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub type SessionId = u32;

/// The boot console. Fixed id because main wires it before anything else exists.
pub const UART_SESSION: SessionId = 0;

const RING_CAP: usize = 16 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChannelKind {
    /// The serial console. Bytes go straight out through the UART driver, which
    /// blocks until they are on the wire. There is exactly one of these and the
    /// driver already buffers, so this arm needs no ring of its own.
    Uart,

    /// One SSH channel. Bytes are parked here and moved onto the wire by the net
    /// task, so this arm has rings and can run out of room.
    Ssh { channel_id: u32 },
}

pub struct SessionChannel {
    pub id: SessionId,
    pub kind: ChannelKind,

    /// wire -> session. Unused by the Uart arm, which reads the FIFO directly.
    to_session: Mutex<VecDeque<u8>>,

    /// session -> wire. Unused by the Uart arm.
    from_session: Mutex<VecDeque<u8>>,

    open: AtomicBool,
}

impl SessionChannel {
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    pub fn close(&self) {
        self.open.store(false, Ordering::Release);
        self.to_session.lock().clear();
        self.from_session.lock().clear();
    }

    /// Accepts as much as fits and reports how much that was, which may be short.
    /// Never blocks and never spins.
    ///
    /// Blocking here would be legal -- vfs::write releases the fd table guard
    /// before it calls write_at -- but it would need a wakeup protocol with the
    /// drain task, and a short count plus a retry one level up buys the same
    /// thing for nothing. Callers that must not lose bytes loop until done and
    /// yield between attempts, which they can only do outside the VFS.
    ///
    /// "Ring full, try again" and "session is gone" MUST NOT both be Ok(0): a
    /// caller looping until it has written everything would spin forever on a
    /// closed channel. So: WouldBlock (EAGAIN) means retry, IoError (EIO) means
    /// stop, which is also what POSIX gives you for writing to a hung-up
    /// terminal.
    pub fn write(&self, buf: &[u8]) -> KResult<usize> {
        if !self.is_open() {
            return Err(KError::IoError);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        match self.kind {
            // uart::write always takes the whole buffer, so this arm never
            // reports WouldBlock.
            ChannelKind::Uart => Ok(uart::write(buf)),
            ChannelKind::Ssh { .. } => match push(&self.from_session, buf) {
                0 => Err(KError::WouldBlock),
                n => Ok(n),
            },
        }
    }

    /// Ok(0) means end of session, matching read(2). "Nothing available yet" is
    /// WouldBlock, not Ok(0), because a channel cannot block waiting for input
    /// and the two are not the same thing to the caller.
    pub fn read(&self, buf: &mut [u8]) -> KResult<usize> {
        if !self.is_open() {
            return Ok(0);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let n = match self.kind {
            ChannelKind::Uart => uart::read(buf),
            ChannelKind::Ssh { .. } => pop(&self.to_session, buf),
        };
        match n {
            0 => Err(KError::WouldBlock),
            n => Ok(n),
        }
    }

    /// SSH receive path: wire bytes headed for this session's stdin.
    pub fn push_input(&self, data: &[u8]) -> usize {
        if !self.is_open() {
            return 0;
        }
        push(&self.to_session, data)
    }

    /// SSH drain, called from the net task: bytes this session wants on the wire.
    pub fn take_output(&self, max: usize) -> Vec<u8> {
        let mut ring = self.from_session.lock();
        let n = core::cmp::min(max, ring.len());
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            match ring.pop_front() {
                Some(b) => out.push(b),
                None => break,
            }
        }
        out
    }

    pub fn has_output(&self) -> bool {
        !self.from_session.lock().is_empty()
    }
}

fn push(ring: &Mutex<VecDeque<u8>>, data: &[u8]) -> usize {
    let mut r = ring.lock();
    let n = core::cmp::min(RING_CAP.saturating_sub(r.len()), data.len());
    r.extend(data[..n].iter().copied());
    n
}

fn pop(ring: &Mutex<VecDeque<u8>>, buf: &mut [u8]) -> usize {
    let mut r = ring.lock();
    let n = core::cmp::min(buf.len(), r.len());
    for slot in buf[..n].iter_mut() {
        match r.pop_front() {
            Some(b) => *slot = b,
            None => break,
        }
    }
    n
}

static SESSIONS: Mutex<BTreeMap<SessionId, Arc<SessionChannel>>> = Mutex::new(BTreeMap::new());

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

pub fn create(kind: ChannelKind) -> Arc<SessionChannel> {
    let id = match kind {
        ChannelKind::Uart => UART_SESSION,
        ChannelKind::Ssh { .. } => NEXT_ID.fetch_add(1, Ordering::Relaxed),
    };
    let chan = Arc::new(SessionChannel {
        id,
        kind,
        to_session: Mutex::new(VecDeque::new()),
        from_session: Mutex::new(VecDeque::new()),
        open: AtomicBool::new(true),
    });
    SESSIONS.lock().insert(id, chan.clone());
    chan
}

pub fn get(id: SessionId) -> Option<Arc<SessionChannel>> {
    SESSIONS.lock().get(&id).cloned()
}

pub fn destroy(id: SessionId) {
    // Take the entry out and release SESSIONS before close() touches the ring
    // locks, so the two are never held at once.
    let chan = SESSIONS.lock().remove(&id);
    if let Some(c) = chan {
        c.close();
    }
}

/// A session's console as a VFS inode.
///
/// It holds the `Arc<SessionChannel>` directly rather than a `SessionId`, so
/// nothing on the fd path ever needs a SESSIONS lookup. That is deliberate:
/// SESSIONS must never be locked while PROCESS_FD_TABLES is held, and capturing
/// the Arc at construction makes that ordering impossible to get wrong later.
pub struct SessionConsole {
    chan: Arc<SessionChannel>,
}

impl SessionConsole {
    pub fn new(chan: Arc<SessionChannel>) -> Arc<dyn Inode> {
        Arc::new(SessionConsole { chan })
    }

    pub fn channel(&self) -> &Arc<SessionChannel> {
        &self.chan
    }
}

impl Inode for SessionConsole {
    fn kind(&self) -> InodeKind {
        InodeKind::Device
    }

    /// A channel has no length. Reporting 0 also keeps vfs::write's O_APPEND path
    /// harmless: it resolves the offset to 0, and read_at/write_at ignore the
    /// offset anyway.
    fn size(&self) -> u64 {
        0
    }

    fn read_at(&self, _off: u64, buf: &mut [u8]) -> KResult<usize> {
        self.chan.read(buf)
    }

    fn write_at(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        self.chan.write(buf)
    }
}
