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

    Signal = 15,

    Kill = 16,

    Sigreturn = 17,

    Socket = 18,

    Bind = 19,

    Listen = 20,

    Accept = 21,

    Connect = 22,

    GetTimeOfDay = 23,

    SetTimeOfDay = 24,

    OtaState = 25,

    Pipe = 26,

    Dup2 = 27,

    Chdir = 28,

    Getcwd = 29,

    Getpgid = 30,

    Setpgid = 31,

    Getpid = 32,

    Getppid = 33,

    Stat = 34,

    Lstat = 35,

    Symlink = 36,

    Readlink = 37,

    Rename = 38,

    Chmod = 39,

    Chown = 40,

    Chroot = 41,

    Flock = 42,

    Mount = 43,

    Umount = 44,

    Tcsetpgrp = 45,

    Getuid = 46,

    Setuid = 47,
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
            15 => Syscall::Signal,
            16 => Syscall::Kill,
            17 => Syscall::Sigreturn,
            18 => Syscall::Socket,
            19 => Syscall::Bind,
            20 => Syscall::Listen,
            21 => Syscall::Accept,
            22 => Syscall::Connect,
            23 => Syscall::GetTimeOfDay,
            24 => Syscall::SetTimeOfDay,
            25 => Syscall::OtaState,
            26 => Syscall::Pipe,
            27 => Syscall::Dup2,
            28 => Syscall::Chdir,
            29 => Syscall::Getcwd,
            30 => Syscall::Getpgid,
            31 => Syscall::Setpgid,
            32 => Syscall::Getpid,
            33 => Syscall::Getppid,
            34 => Syscall::Stat,
            35 => Syscall::Lstat,
            36 => Syscall::Symlink,
            37 => Syscall::Readlink,
            38 => Syscall::Rename,
            39 => Syscall::Chmod,
            40 => Syscall::Chown,
            41 => Syscall::Chroot,
            42 => Syscall::Flock,
            43 => Syscall::Mount,
            44 => Syscall::Umount,
            45 => Syscall::Tcsetpgrp,
            46 => Syscall::Getuid,
            47 => Syscall::Setuid,
            _ => return None,
        };
        Some(sc)
    }

    pub const fn number(self) -> usize {
        self as usize
    }
}
