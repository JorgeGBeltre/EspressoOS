#![no_std]
#![no_main]

use libc::{arg, println, readdir};

/// Lists one directory. Entries are `[ino: u64][kind: u8][name_len: u16][name]`.
fn list(path: &str) -> i32 {
    let mut buf = [0u8; 1024];
    let n = readdir(path, &mut buf);
    if n < 0 {
        println!("ls: {}: cannot read", path);
        return 1;
    }

    let limit = n as usize;
    let mut pos = 0;
    while pos + 11 <= limit {
        let _ino = u64::from_le_bytes([
            buf[pos],
            buf[pos + 1],
            buf[pos + 2],
            buf[pos + 3],
            buf[pos + 4],
            buf[pos + 5],
            buf[pos + 6],
            buf[pos + 7],
        ]);
        let _kind = buf[pos + 8];
        let name_len = u16::from_le_bytes([buf[pos + 9], buf[pos + 10]]) as usize;
        pos += 11;

        if pos + name_len > limit {
            break;
        }
        if let Ok(name) = core::str::from_utf8(&buf[pos..pos + name_len]) {
            println!("{}", name);
        }
        pos += name_len;
    }
    0
}

/// ls(1). Takes paths now; it used to read "/" no matter what was asked, which made
/// `ls /tmp` and `ls /tmp | cat` disagree -- the first is the shell built-in and
/// honours the path, the second is this and did not.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        // "." -- the working directory, which this program still cannot see and does
        // not need to. There is no getcwd syscall; the VFS resolves the "." against
        // the cwd this process inherited when it was spawned. That is the whole point
        // of the invariant: userland names a directory relative to itself and the
        // kernel knows which one that is.
        //
        // This said "/" until the VFS learned to resolve relative paths, and the
        // comment here named that as the fix. It is why `cd /tmp` then `ls` and
        // `cd /tmp` then `/bin/ls` used to disagree.
        return list(".");
    }

    let mut status = 0;
    for i in 1..argc {
        let path = unsafe { arg(argv, i) };
        // Only label each listing when there is more than one, the way ls does.
        if argc > 2 {
            println!("{}:", path);
        }
        if list(path) != 0 {
            status = 1;
        }
    }
    status
}
