use super::inode::{DirEntry, Inode, InodeKind};
use crate::arch::xtensa::sync::Mutex as KMutex;
use crate::prelude::*;
use smoltcp::iface::SocketHandle;

pub struct SocketInode {
    pub handle: KMutex<SocketHandle>,
    pub is_udp: bool,
    pub remote_endpoint: KMutex<Option<smoltcp::wire::IpEndpoint>>,
    pub local_port: KMutex<Option<u16>>,
}

impl Inode for SocketInode {
    fn kind(&self) -> InodeKind {
        InodeKind::File
    }

    fn size(&self) -> u64 {
        0
    }

    fn as_socket(&self) -> Option<SocketHandle> {
        Some(*self.handle.lock())
    }

    fn is_udp_socket(&self) -> bool {
        self.is_udp
    }

    fn set_socket_remote_endpoint(&self, endpoint: smoltcp::wire::IpEndpoint) -> KResult<()> {
        *self.remote_endpoint.lock() = Some(endpoint);
        Ok(())
    }

    fn bind(&self, port: u16) -> KResult<()> {
        *self.local_port.lock() = Some(port);
        let handle = *self.handle.lock();
        if self.is_udp {
            let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
            if let Some(sockets) = guard.as_mut() {
                let sock = sockets.get_mut::<smoltcp::socket::udp::Socket>(handle);
                sock.bind(port).map_err(|_| KError::InvalidArgument)?;
            }
        }
        Ok(())
    }

    fn listen(&self, _backlog: i32) -> KResult<()> {
        if self.is_udp {
            return Err(KError::NotSupported);
        }
        let port = self.local_port.lock().ok_or(KError::InvalidArgument)?;
        let handle = *self.handle.lock();
        let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
        if let Some(sockets) = guard.as_mut() {
            let sock = sockets.get_mut::<smoltcp::socket::tcp::Socket>(handle);
            sock.listen(port).map_err(|_| KError::InvalidArgument)?;
        }
        Ok(())
    }

    fn accept(&self) -> KResult<Arc<dyn Inode>> {
        if self.is_udp {
            return Err(KError::NotSupported);
        }

        let handle = *self.handle.lock();

        loop {
            let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
            if let Some(sockets) = guard.as_mut() {
                let sock = sockets.get_mut::<smoltcp::socket::tcp::Socket>(handle);
                if sock.is_active() && sock.state() == smoltcp::socket::tcp::State::Established {
                    break;
                }
                if sock.state() == smoltcp::socket::tcp::State::Closed {
                    return Err(KError::IoError);
                }
            }
            drop(guard);
            crate::scheduler::yield_now();
        }

        let local_port = self.local_port.lock().ok_or(KError::InvalidArgument)?;

        let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
        let sockets = guard.as_mut().ok_or(KError::IoError)?;

        let rx_buf = smoltcp::socket::tcp::SocketBuffer::new(alloc::vec![0; 4096]);
        let tx_buf = smoltcp::socket::tcp::SocketBuffer::new(alloc::vec![0; 4096]);
        let mut new_sock = smoltcp::socket::tcp::Socket::new(rx_buf, tx_buf);
        new_sock.listen(local_port).map_err(|_| KError::IoError)?;
        let new_handle = sockets.add(new_sock);

        let mut current_handle_guard = self.handle.lock();
        let connected_handle = *current_handle_guard;
        *current_handle_guard = new_handle;

        let accepted_inode = Arc::new(SocketInode {
            handle: KMutex::new(connected_handle),
            is_udp: false,
            remote_endpoint: KMutex::new(None),
            local_port: KMutex::new(Some(local_port)),
        });
        Ok(accepted_inode)
    }

    fn read_at(&self, _off: u64, buf: &mut [u8]) -> KResult<usize> {
        let handle = *self.handle.lock();
        loop {
            let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
            if let Some(sockets) = guard.as_mut() {
                if self.is_udp {
                    let sock = sockets.get_mut::<smoltcp::socket::udp::Socket>(handle);
                    if sock.can_recv() {
                        if let Ok((n, _remote_ep)) = sock.recv_slice(buf) {
                            return Ok(n);
                        }
                    }
                } else {
                    let sock = sockets.get_mut::<smoltcp::socket::tcp::Socket>(handle);
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
        let handle = *self.handle.lock();
        loop {
            let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
            if let Some(sockets) = guard.as_mut() {
                if self.is_udp {
                    let sock = sockets.get_mut::<smoltcp::socket::udp::Socket>(handle);
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
                    let sock = sockets.get_mut::<smoltcp::socket::tcp::Socket>(handle);
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
        let handle = *self.handle.lock();
        let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
        if let Some(sockets) = guard.as_mut() {
            if !self.is_udp {
                let sock = sockets.get_mut::<smoltcp::socket::tcp::Socket>(handle);
                sock.close();
            }
            sockets.remove(handle);
        }
    }
}
