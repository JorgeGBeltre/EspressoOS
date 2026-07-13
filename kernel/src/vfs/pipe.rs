#![allow(dead_code)]

use crate::prelude::*;
use crate::arch::xtensa::sync::Mutex;
use super::inode::{Inode, InodeKind, DirEntry};
use core::sync::atomic::{AtomicUsize, Ordering};

pub struct Pipe {
    buffer: Mutex<Vec<u8>>,
    capacity: usize,
    readers_blocked: Mutex<Vec<crate::scheduler::task::Tid>>,
    writers_blocked: Mutex<Vec<crate::scheduler::task::Tid>>,
    pub reader_count: AtomicUsize,
    pub writer_count: AtomicUsize,
}

impl Pipe {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            buffer: Mutex::new(Vec::new()),
            capacity,
            readers_blocked: Mutex::new(Vec::new()),
            writers_blocked: Mutex::new(Vec::new()),
            reader_count: AtomicUsize::new(1),
            writer_count: AtomicUsize::new(1),
        })
    }
}

pub struct PipeReadInode {
    pipe: Arc<Pipe>,
}

impl PipeReadInode {
    pub fn new(pipe: Arc<Pipe>) -> Self {
        Self { pipe }
    }
}

impl Inode for PipeReadInode {
    fn kind(&self) -> InodeKind {
        InodeKind::File
    }

    fn size(&self) -> u64 {
        self.pipe.buffer.lock().len() as u64
    }

    fn read_at(&self, _off: u64, buf: &mut [u8]) -> KResult<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        loop {
            let mut guard = self.pipe.buffer.lock();
            if !guard.is_empty() {
                let n = core::cmp::min(buf.len(), guard.len());
                for (i, b) in guard.drain(0..n).enumerate() {
                    buf[i] = b;
                }
                
                // Despertar escritores bloqueados
                let mut writers = self.pipe.writers_blocked.lock();
                for &tid in writers.iter() {
                    crate::scheduler::unblock_task(tid);
                }
                writers.clear();
                
                return Ok(n);
            }
            
            // Si el buffer está vacío y no hay escritores, retorna EOF (0)
            if self.pipe.writer_count.load(Ordering::SeqCst) == 0 {
                return Ok(0);
            }
            
            // Bloquear lector actual
            let tid = crate::scheduler::current();
            self.pipe.readers_blocked.lock().push(tid);
            
            drop(guard);
            crate::scheduler::block_current();
        }
    }

    fn write_at(&self, _off: u64, _buf: &[u8]) -> KResult<usize> {
        Err(KError::PermissionDenied)
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

impl Drop for PipeReadInode {
    fn drop(&mut self) {
        if self.pipe.reader_count.fetch_sub(1, Ordering::SeqCst) == 1 {
            // Último lector cerrado: despertar a los escritores bloqueados (que fallarán con EPIPE/IoError)
            let mut writers = self.pipe.writers_blocked.lock();
            for &tid in writers.iter() {
                crate::scheduler::unblock_task(tid);
            }
            writers.clear();
        }
    }
}

pub struct PipeWriteInode {
    pipe: Arc<Pipe>,
}

impl PipeWriteInode {
    pub fn new(pipe: Arc<Pipe>) -> Self {
        Self { pipe }
    }
}

impl Inode for PipeWriteInode {
    fn kind(&self) -> InodeKind {
        InodeKind::File
    }

    fn size(&self) -> u64 {
        self.pipe.buffer.lock().len() as u64
    }

    fn read_at(&self, _off: u64, _buf: &mut [u8]) -> KResult<usize> {
        Err(KError::PermissionDenied)
    }

    fn write_at(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        loop {
            // Si no hay lectores, escribir genera EPIPE (IoError)
            if self.pipe.reader_count.load(Ordering::SeqCst) == 0 {
                return Err(KError::IoError);
            }

            let mut guard = self.pipe.buffer.lock();
            if guard.len() < self.pipe.capacity {
                let space = self.pipe.capacity - guard.len();
                let n = core::cmp::min(buf.len(), space);
                guard.extend_from_slice(&buf[0..n]);
                
                // Despertar lectores bloqueados
                let mut readers = self.pipe.readers_blocked.lock();
                for &tid in readers.iter() {
                    crate::scheduler::unblock_task(tid);
                }
                readers.clear();
                
                return Ok(n);
            }
            
            // Buffer lleno: bloquear escritor actual
            let tid = crate::scheduler::current();
            self.pipe.writers_blocked.lock().push(tid);
            
            drop(guard);
            crate::scheduler::block_current();
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

impl Drop for PipeWriteInode {
    fn drop(&mut self) {
        if self.pipe.writer_count.fetch_sub(1, Ordering::SeqCst) == 1 {
            // Último escritor cerrado: despertar a los lectores bloqueados (que recibirán EOF/0)
            let mut readers = self.pipe.readers_blocked.lock();
            for &tid in readers.iter() {
                crate::scheduler::unblock_task(tid);
            }
            readers.clear();
        }
    }
}

/// Crea una tubería unidireccional y devuelve los Inodes de lectura y escritura.
pub fn create_pipe(capacity: usize) -> (Arc<dyn Inode>, Arc<dyn Inode>) {
    let pipe = Pipe::new(capacity);
    let read_inode = Arc::new(PipeReadInode::new(pipe.clone()));
    let write_inode = Arc::new(PipeWriteInode::new(pipe));
    (read_inode, write_inode)
}
