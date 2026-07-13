#![allow(dead_code)]

#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Syscall {

    Read = 0,

    Write = 1,

    Open = 2,

    Close = 3,

    Ioctl = 4,

    Exit = 5,

    Spawn = 6,

    Wait = 7,

    Seek = 8,

    Mkdir = 9,

    Unlink = 10,

    Readdir = 11,

    UptimeMs = 12,

    Sbrk = 13,

    Yield = 14,
}

impl Syscall {

    pub fn from_usize(n: usize) -> Option<Syscall> {
        let sc = match n {
            0 => Syscall::Read,
            1 => Syscall::Write,
            2 => Syscall::Open,
            3 => Syscall::Close,
            4 => Syscall::Ioctl,
            5 => Syscall::Exit,
            6 => Syscall::Spawn,
            7 => Syscall::Wait,
            8 => Syscall::Seek,
            9 => Syscall::Mkdir,
            10 => Syscall::Unlink,
            11 => Syscall::Readdir,
            12 => Syscall::UptimeMs,
            13 => Syscall::Sbrk,
            14 => Syscall::Yield,
            _ => return None,
        };
        Some(sc)
    }

    pub const fn number(self) -> usize {
        self as usize
    }
}
