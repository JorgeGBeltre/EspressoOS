use crate::prelude::*;
use super::inode::{Inode, InodeKind, DirEntry};
use smoltcp::iface::SocketHandle;
use crate::arch::xtensa::sync::Mutex as KMutex;

pub struct SocketInode {
    pub handle: SocketHandle,
    pub is_udp: bool,
    pub remote_endpoint: KMutex<Option<smoltcp::wire::IpEndpoint>>,
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

    fn is_udp_socket(&self) -> bool {
        self.is_udp
    }

    fn set_socket_remote_endpoint(&self, endpoint: smoltcp::wire::IpEndpoint) -> KResult<()> {
        *self.remote_endpoint.lock() = Some(endpoint);
        Ok(())
    }

    fn read_at(&self, _off: u64, buf: &mut [u8]) -> KResult<usize> {
        loop {
            let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
            if let Some(sockets) = guard.as_mut() {
                if self.is_udp {
                    let sock = sockets.get_mut::<smoltcp::socket::udp::Socket>(self.handle);
                    if sock.can_recv() {
                        if let Ok((n, _remote_ep)) = sock.recv_slice(buf) {
                            return Ok(n);
                        }
                    }
                } else {
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
            }
            drop(guard);
            crate::scheduler::yield_now();
        }
    }

    fn write_at(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        loop {
            let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
            if let Some(sockets) = guard.as_mut() {
                if self.is_udp {
                    let sock = sockets.get_mut::<smoltcp::socket::udp::Socket>(self.handle);
                    let ep_guard = self.remote_endpoint.lock();
                    if let Some(ep) = *ep_guard {
                        if sock.can_send() {
                            if let Ok(_) = sock.send_slice(buf, ep) {
                                return Ok(buf.len());
                            }
                        }
                    } else {
                        return Err(KError::InvalidArgument);
                    }
                } else {
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
            if !self.is_udp {
                let sock = sockets.get_mut::<smoltcp::socket::tcp::Socket>(self.handle);
                sock.close();
            }
            sockets.remove(self.handle);
        }
    }
}
