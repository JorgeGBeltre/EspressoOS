#![no_std]
#![no_main]

use libc::{close, dup2, print, println, read, readdir, spawn, wait, yield_now, pipe};

/// Most tokens one command can have, counting argv[0] and the trailing NULL.
const MAX_ARGV: usize = 16;

/// Longest command line accepted. `LINE + 1` bytes are kept: `tokenize` writes the
/// last token's NUL one past the end of the range it is given.
const LINE: usize = 64;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    println!("--- EspressoOS Shell (Userland) ---");
    let mut buf = [0u8; LINE + 1];
    loop {
        print!("EspressoOS:~$ ");
        let len = read_line(&mut buf[..LINE]);
        let (s, e) = trim(&buf, 0, len);
        if s == e {
            continue;
        }

        if &buf[s..e] == b"exit" {
            println!("Exiting the shell...");
            break;
        }
        if &buf[s..e] == b"help" {
            print_help();
            continue;
        }

        match buf[s..e].iter().position(|&c| c == b'|') {
            Some(k) => run_pipeline(&mut buf, (s, s + k), (s + k + 1, e)),
            None => run_single(&mut buf, (s, e)),
        }
    }
    0
}

/// Reads one line into `buf`, echoing as it goes. Returns its length.
fn read_line(buf: &mut [u8]) -> usize {
    let mut len = 0;
    loop {
        let mut c = [0u8; 1];
        if read(0, &mut c) <= 0 {
            yield_now();
            continue;
        }
        if c[0] == b'\n' || c[0] == b'\r' {
            println!("");
            return len;
        }
        let _ = libc::write(1, &c);
        if len < buf.len() {
            buf[len] = c[0];
            len += 1;
        }
    }
}

/// `start..end` with leading and trailing blanks removed.
fn trim(buf: &[u8], mut start: usize, mut end: usize) -> (usize, usize) {
    while start < end && (buf[start] == b' ' || buf[start] == b'\t') {
        start += 1;
    }
    while end > start && (buf[end - 1] == b' ' || buf[end - 1] == b'\t') {
        end -= 1;
    }
    (start, end)
}

/// Splits `buf[start..end]` into NUL-terminated tokens in place and fills `argv` with
/// pointers to them, NULL-terminated. Returns the token count.
///
/// The separators become the terminators, so each token ends up a C string pointing
/// into `buf` -- nothing is copied and nothing is allocated, which matters because
/// there is no allocator here. The kernel copies the strings out during `spawn`, so
/// they only have to survive that one call.
///
/// `buf[end]` is overwritten with the last token's NUL. That is why the line buffer
/// is one byte longer than the longest line it accepts.
///
/// None means the line has more tokens than `argv` can hold.
fn tokenize(
    buf: &mut [u8],
    start: usize,
    end: usize,
    argv: &mut [*const u8; MAX_ARGV],
) -> Option<usize> {
    buf[end] = 0;

    let mut offs = [0usize; MAX_ARGV];
    let mut n = 0;
    let mut i = start;
    while i < end {
        while i < end && buf[i] == b' ' {
            i += 1;
        }
        if i >= end {
            break;
        }
        // argv[n] takes the token, so argv[n + 1] has to be free for the NULL.
        if n + 1 >= MAX_ARGV {
            return None;
        }
        offs[n] = i;
        n += 1;
        while i < end && buf[i] != b' ' {
            i += 1;
        }
        if i < end {
            buf[i] = 0;
            i += 1;
        }
    }

    // Pointers are taken only once every terminator is in, so nothing derived here
    // is invalidated by a later write through the slice.
    let base = buf.as_ptr();
    for k in 0..n {
        argv[k] = unsafe { base.add(offs[k]) };
    }
    argv[n] = core::ptr::null();
    Some(n)
}

/// Tokenizes a command and resolves its program, reporting why if it cannot.
///
/// Returns the token count; `argv` is left NULL-terminated and ready for `spawn`.
fn prepare<'a>(
    buf: &mut [u8],
    range: (usize, usize),
    argv: &mut [*const u8; MAX_ARGV],
    path_buf: &'a mut [u8],
) -> Option<&'a str> {
    match tokenize(buf, range.0, range.1, argv) {
        None => {
            println!("Too many arguments (max {})", MAX_ARGV - 1);
            return None;
        }
        Some(0) => {
            println!("Empty command");
            return None;
        }
        Some(_) => {}
    }

    // argv[0] is the name as typed; the path is where it was found. They differ for
    // anything run by bare name, and the child is meant to see the former.
    let path = resolve_path(unsafe { libc::arg(argv.as_ptr(), 0) }, path_buf);
    if path.is_empty() {
        println!("Invalid command path");
        return None;
    }
    Some(path)
}

fn run_single(buf: &mut [u8], range: (usize, usize)) {
    let mut argv = [core::ptr::null(); MAX_ARGV];
    let mut path_buf = [0u8; LINE + 8];
    let path = match prepare(buf, range, &mut argv, &mut path_buf) {
        Some(p) => p,
        None => return,
    };

    if spawn(path, argv.as_ptr()) < 0 {
        println!("Error executing: {}", path);
        return;
    }
    let mut status = 0;
    let _ = wait(&mut status);
}

fn run_pipeline(buf: &mut [u8], left: (usize, usize), right: (usize, usize)) {
    let mut argv1 = [core::ptr::null(); MAX_ARGV];
    let mut argv2 = [core::ptr::null(); MAX_ARGV];
    let mut path_buf1 = [0u8; LINE + 8];
    let mut path_buf2 = [0u8; LINE + 8];

    // Both stages are prepared before a single fd is touched. The two ranges are
    // disjoint so neither pass disturbs the other's tokens, and a command that cannot
    // run is reported while there is still nothing to unwind.
    let path1 = match prepare(buf, left, &mut argv1, &mut path_buf1) {
        Some(p) => p,
        None => return,
    };
    let path2 = match prepare(buf, right, &mut argv2, &mut path_buf2) {
        Some(p) => p,
        None => return,
    };

    let mut p = [0i32; 2];
    if pipe(&mut p) < 0 {
        println!("Error creating pipe");
        return;
    }

    let saved_stdout = dup2(1, 10);
    let saved_stdin = dup2(0, 11);

    // Stage 1 writes into the pipe.
    dup2(p[1], 1);
    let pid1 = spawn(path1, argv1.as_ptr());

    // Every copy of the write end has to leave THIS table before stage 2 is spawned.
    // spawn hands a child a clone of the whole fd table, and the pipe reports EOF
    // only once the last reference to its write inode is dropped -- by anyone, in any
    // process, since they all share one. A stage 2 that inherits one is a writer to
    // the pipe it is reading, and its read blocks forever however promptly stage 1
    // exits. Closing these after both spawns, which is where they used to be, is too
    // late by exactly one clone.
    dup2(saved_stdout as i32, 1);
    close(p[1]);

    // Stage 2 reads from it.
    dup2(p[0], 0);
    let pid2 = spawn(path2, argv2.as_ptr());

    dup2(saved_stdin as i32, 0);
    close(p[0]);
    close(10);
    close(11);

    if pid1 >= 0 && pid2 >= 0 {
        let mut status = 0;
        let _ = wait(&mut status);
        let _ = wait(&mut status);
    } else {
        println!("Error spawning pipeline processes");
    }
}

fn print_help() {
    println!("EspressoOS userland shell -- built-in commands:");
    println!("  help                 show this help");
    println!("  exit                 exit the shell");
    println!("  <cmd> | <cmd>        pipe one command's output into another");
    println!("");
    println!("Programs take arguments: 'echo hola mundo', 'ls /tmp'.");
    println!("");
    println!("Programs in /bin (run by name, e.g. 'ls' or '/bin/ls'):");
    let mut buf = [0u8; 1024];
    let n = readdir("/bin", &mut buf);
    if n < 0 {
        println!("  (could not read /bin)");
        return;
    }
    let mut pos = 0;
    let limit = n as usize;
    while pos < limit {
        if pos + 11 > limit {
            break;
        }
        let name_len = u16::from_le_bytes([buf[pos + 9], buf[pos + 10]]) as usize;
        pos += 11;
        if pos + name_len > limit {
            break;
        }
        if let Ok(name) = core::str::from_utf8(&buf[pos..pos + name_len]) {
            println!("  {}", name);
        }
        pos += name_len;
    }
}

/// Where to look for `cmd`: /bin if it is a bare name, otherwise wherever it says.
///
/// This is PATH search and nothing else. The /bin prefix must stay -- `ls` means the
/// program, and dropping it because "the VFS resolves paths now" would send `ls` after
/// a `cd /tmp` looking for /tmp/ls. But it applies only to a bare NAME.
///
/// The test is `contains('/')`, not `starts_with('/')`, which is what the kernel shell
/// has always used. With starts_with, `./hello` was not absolute, so it got the prefix:
/// "/bin/./hello", which normalize collapses to "/bin/hello". Typing ./hello in /tmp
/// ran a different program with the same name and said nothing, or reported "not found"
/// about a path never typed. /tmp/hello was never consulted.
fn resolve_path<'a>(cmd: &str, out_buf: &'a mut [u8]) -> &'a str {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return "";
    }
    if cmd.contains('/') {
        let cmd_bytes = cmd.as_bytes();
        if cmd_bytes.len() < out_buf.len() {
            out_buf[..cmd_bytes.len()].copy_from_slice(cmd_bytes);
            core::str::from_utf8(&out_buf[..cmd_bytes.len()]).unwrap_or("")
        } else {
            ""
        }
    } else {
        let prefix = b"/bin/";
        let cmd_bytes = cmd.as_bytes();
        if prefix.len() + cmd_bytes.len() < out_buf.len() {
            out_buf[..prefix.len()].copy_from_slice(prefix);
            out_buf[prefix.len()..prefix.len() + cmd_bytes.len()].copy_from_slice(cmd_bytes);
            core::str::from_utf8(&out_buf[..prefix.len() + cmd_bytes.len()]).unwrap_or("")
        } else {
            ""
        }
    }
}
