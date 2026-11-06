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

    /// Who authenticated on this session, or None for the console, which has no
    /// login. A task entry takes a usize and a name is a String, so this is how the
    /// user reaches the shell: the task gets its session id and looks itself up.
    pub user: Option<String>,

    /// wire -> session. Unused by the Uart arm, which reads the FIFO directly.
    to_session: Mutex<VecDeque<u8>>,

    /// session -> wire. Unused by the Uart arm.
    from_session: Mutex<VecDeque<u8>>,

    /// Foreground process group id for TTY job control (`0` = interactive shell owns TTY).
    fg_pgid: AtomicU32,

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
        // ONLCR lives here, in the terminal, not in the shell. Callers emit a bare
        // \n and a channel turns it into \r\n on the way out; a file inode does no
        // such thing, so `echo x > f` lands as LF the way it should. That also
        // stops redirection from being a special case in the shell.
        match self.kind {
            // uart::write always takes the whole slice, so this arm can neither
            // tear a \r\n nor report WouldBlock.
            ChannelKind::Uart => {
                let mut rx = [0u8; 64];
                let got = uart::read(&mut rx);
                if got > 0 {
                    let _ = push(&self.to_session, &rx[..got]);
                }
                for (body, nl) in onlcr_chunks(buf) {
                    if !body.is_empty() {
                        uart::write(body);
                    }
                    if nl {
                        uart::write(b"\r\n");
                    }
                    let got2 = uart::read(&mut rx);
                    if got2 > 0 {
                        let _ = push(&self.to_session, &rx[..got2]);
                    }
                }
                Ok(buf.len())
            }
            ChannelKind::Ssh { .. } => {
                let mut ring = self.from_session.lock();
                let mut taken = 0usize;
                for (body, nl) in onlcr_chunks(buf) {
                    let n = core::cmp::min(RING_CAP.saturating_sub(ring.len()), body.len());
                    ring.extend(body[..n].iter().copied());
                    taken += n;
                    if n < body.len() {
                        break;
                    }
                    if nl {
                        // All or nothing: half a \r\n on the wire is worse than
                        // none, so leave the \n for the caller's next attempt.
                        if RING_CAP.saturating_sub(ring.len()) < 2 {
                            break;
                        }
                        ring.extend(b"\r\n".iter().copied());
                        // The caller handed us one byte, not the two we sent.
                        taken += 1;
                    }
                }
                match taken {
                    0 => Err(KError::WouldBlock),
                    n => Ok(n),
                }
            }
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
            ChannelKind::Uart => {
                let mut got = pop(&self.to_session, buf);
                if got < buf.len() {
                    got += uart::read(&mut buf[got..]);
                }
                got
            }
            ChannelKind::Ssh { .. } => pop(&self.to_session, buf),
        };
        if n == 0 {
            return Err(KError::WouldBlock);
        }
        let fg = self.fg_pgid.load(Ordering::Acquire);
        if fg != 0 {
            let mut sig = 0i32;
            let mut keep = 0usize;
            for i in 0..n {
                let b = buf[i];
                if b == 0x03 {
                    sig = 2;
                } else if b == 0x1A {
                    sig = 20;
                } else {
                    buf[keep] = b;
                    keep += 1;
                }
            }
            if sig != 0 {
                let _ = self.write(b"^C\n");
                deliver_signal_to_pgid(fg, sig);
            }
            if keep == 0 {
                return Err(KError::WouldBlock);
            }
            return Ok(keep);
        }
        Ok(n)
    }

    pub fn set_fg_pgid(&self, pgid: u32) {
        self.fg_pgid.store(pgid, Ordering::Release);
    }

    pub fn fg_pgid(&self) -> u32 {
        self.fg_pgid.load(Ordering::Acquire)
    }

    /// SSH receive path: wire bytes headed for this session's stdin.
    pub fn push_input(&self, data: &[u8]) -> usize {
        if !self.is_open() {
            return 0;
        }
        let fg = self.fg_pgid.load(Ordering::Acquire);
        if fg != 0 && data.iter().any(|&b| b == 0x03 || b == 0x1A) {
            let mut sig = 0i32;
            let mut filtered = Vec::with_capacity(data.len());
            for &b in data {
                if b == 0x03 {
                    sig = 2;
                } else if b == 0x1A {
                    sig = 20;
                } else {
                    filtered.push(b);
                }
            }
            let _ = push(&self.to_session, &filtered);
            if sig != 0 {
                let _ = self.write(b"^C\n");
                deliver_signal_to_pgid(fg, sig);
            }
            return data.len();
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

/// Splits `buf` into (body, ends_with_newline) pairs for ONLCR.
///
/// Allocates nothing and copies nothing: split_inclusive keeps each \n with its
/// own chunk, and a buffer with no \n at all is one chunk and one send.
fn onlcr_chunks(buf: &[u8]) -> impl Iterator<Item = (&[u8], bool)> {
    buf.split_inclusive(|&b| b == b'\n')
        .map(|chunk| match chunk.strip_suffix(b"\n") {
            Some(body) => (body, true),
            None => (chunk, false),
        })
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

pub fn create(kind: ChannelKind, user: Option<String>) -> Arc<SessionChannel> {
    let id = match kind {
        ChannelKind::Uart => UART_SESSION,
        ChannelKind::Ssh { .. } => NEXT_ID.fetch_add(1, Ordering::Relaxed),
    };
    let chan = Arc::new(SessionChannel {
        id,
        kind,
        user,
        to_session: Mutex::new(VecDeque::new()),
        from_session: Mutex::new(VecDeque::new()),
        fg_pgid: AtomicU32::new(0),
        open: AtomicBool::new(true),
    });
    SESSIONS.lock().insert(id, chan.clone());
    chan
}

pub fn deliver_signal_to_pgid(pgid: u32, sig: i32) {
    if pgid == 0 || sig <= 0 || sig >= 32 {
        return;
    }
    let mut pt = crate::scheduler::process::PROCESS_TABLE.lock();
    let mut matched = false;
    for proc in pt.table.values_mut() {
        if proc.pgid == pgid {
            proc.pending_signals |= 1 << sig;
            crate::scheduler::unblock_task(proc.main_task);
            matched = true;
        }
    }
    if matched {
        crate::scheduler::set_need_resched();
    }
}

/// Polls UART0 RX FIFO when the serial console has an active foreground job (`fg_pgid != 0`),
/// delivering `SIGINT`/`SIGTSTP` on `Ctrl+C`/`Ctrl+Z` while stashing ordinary input bytes.
pub fn poll_uart_signals() {
    let chan = match get(UART_SESSION) {
        Some(c) => c,
        None => return,
    };
    let fg = chan.fg_pgid();
    if fg == 0 {
        return;
    }
    let mut raw = [0u8; 32];
    let n = uart::read(&mut raw);
    if n == 0 {
        return;
    }
    let mut sig = 0i32;
    for &b in &raw[..n] {
        if b == 0x03 {
            sig = 2;
        } else if b == 0x1A {
            sig = 20;
        } else {
            let _ = push(&chan.to_session, &[b]);
        }
    }
    if sig != 0 {
        let _ = chan.write(b"^C\n");
        deliver_signal_to_pgid(fg, sig);
    }
}

pub fn set_session_fg_pgid(id: SessionId, pgid: u32) -> KResult<()> {
    let chan = get(id).ok_or(KError::NotFound)?;
    chan.set_fg_pgid(pgid);
    Ok(())
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

pub fn list_sessions() -> Vec<(SessionId, String, &'static str, bool)> {
    let map = SESSIONS.lock();
    let mut out = Vec::new();
    for (&id, chan) in map.iter() {
        let user = chan.user.clone().unwrap_or_else(|| String::from("console"));
        let kind_str = match chan.kind {
            ChannelKind::Uart => "tty0",
            ChannelKind::Ssh { .. } => "pts/0",
        };
        out.push((id, user, kind_str, chan.is_open()));
    }
    out
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
