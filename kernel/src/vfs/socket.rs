use crate::prelude::*;
use super::inode::{Inode, InodeKind, DirEntry};
use smoltcp::iface::SocketHandle;

pub struct SocketInode {
    pub handle: SocketHandle,
}

impl Inode for SocketInode {
    fn kind(&self) -> InodeKind {
        InodeKind::File
    }

    fn size(&self) -> u64 {
        0
    }

    fn as_socket(&self) -> Option<SocketHandle> {
        Some(self.handle)
    }

    fn read_at(&self, _off: u64, buf: &mut [u8]) -> KResult<usize> {
        loop {
            let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
            if let Some(sockets) = guard.as_mut() {
                let sock = sockets.get_mut::<smoltcp::socket::tcp::Socket>(self.handle);
                
                if sock.can_recv() {
                    if let Ok(n) = sock.recv_slice(buf) {
                        return Ok(n);
                    }
                }
                
                if !sock.is_open() || sock.state() == smoltcp::socket::tcp::State::CloseWait {
                    return Ok(0);
                }
            }
            drop(guard);
            crate::scheduler::yield_now();
        }
    }

    fn write_at(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        loop {
            let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
            if let Some(sockets) = guard.as_mut() {
                let sock = sockets.get_mut::<smoltcp::socket::tcp::Socket>(self.handle);
                
                if sock.can_send() {
                    if let Ok(n) = sock.send_slice(buf) {
                        return Ok(n);
                    }
                }
                
                if !sock.is_open() {
                    return Err(KError::IoError);
                }
            }
            drop(guard);
            crate::scheduler::yield_now();
        }
    }

    fn readdir(&self, _index: usize) -> KResult<Option<DirEntry>> {
        Err(KError::NotADirectory)
    }

    fn lookup(&self, _name: &str) -> KResult<Arc<dyn Inode>> {
        Err(KError::NotADirectory)
    }

    fn create(&self, _name: &str, _kind: InodeKind) -> KResult<Arc<dyn Inode>> {
        Err(KError::PermissionDenied)
    }

    fn unlink(&self, _name: &str) -> KResult<()> {
        Err(KError::PermissionDenied)
    }
}

impl Drop for SocketInode {
    fn drop(&mut self) {
        let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
        if let Some(sockets) = guard.as_mut() {
            let sock = sockets.get_mut::<smoltcp::socket::tcp::Socket>(self.handle);
            sock.close();
            sockets.remove(self.handle);
        }
    }
}
