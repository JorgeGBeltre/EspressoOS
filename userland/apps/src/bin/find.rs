#![no_std]
#![no_main]

use libc::{arg, println, readdir};

const MAX_QUEUE: usize = 16;
const MAX_PATH: usize = 64;

static mut QUEUE: [[u8; MAX_PATH]; MAX_QUEUE] = [[0; MAX_PATH]; MAX_QUEUE];
static mut QUEUE_LENS: [usize; MAX_QUEUE] = [0; MAX_QUEUE];
static mut QUEUE_HEAD: usize = 0;
static mut QUEUE_TAIL: usize = 0;
static mut DIR_BUF: [u8; 1024] = [0; 1024];

fn push_queue(path: &str) -> bool {
    unsafe {
        let next = (QUEUE_TAIL + 1) % MAX_QUEUE;
        if next == QUEUE_HEAD {
            return false;
        }
        let bytes = path.as_bytes();
        let len = bytes.len().min(MAX_PATH);
        QUEUE[QUEUE_TAIL][..len].copy_from_slice(&bytes[..len]);
        QUEUE_LENS[QUEUE_TAIL] = len;
        QUEUE_TAIL = next;
        true
    }
}

fn pop_queue() -> Option<&'static str> {
    unsafe {
        if QUEUE_HEAD == QUEUE_TAIL {
            return None;
        }
        let idx = QUEUE_HEAD;
        QUEUE_HEAD = (QUEUE_HEAD + 1) % MAX_QUEUE;
        let len = QUEUE_LENS[idx];
        core::str::from_utf8(&QUEUE[idx][..len]).ok()
    }
}

fn pattern_matches(name: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    if pattern.starts_with('*') {
        let suffix = &pattern[1..];
        name.ends_with(suffix)
    } else if pattern.ends_with('*') {
        let prefix = &pattern[..pattern.len() - 1];
        name.starts_with(prefix)
    } else {
        name == pattern
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut root = ".";
    let mut pattern = "";
    let mut i = 1;

    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-name" && i + 1 < argc {
            i += 1;
            pattern = unsafe { arg(argv, i) };
        } else if !a.starts_with('-') {
            root = a;
        }
        i += 1;
    }

    if pattern_matches(root, pattern) {
        println!("{}", root);
    }

    push_queue(root);

    while let Some(current_dir) = pop_queue() {
        let buf = unsafe { &mut DIR_BUF };
        let n = readdir(current_dir, buf);
        if n <= 0 {
            continue;
        }

        let limit = n as usize;
        let mut pos = 0;
        while pos + 11 <= limit {
            let kind = buf[pos + 8];
            let name_len = u16::from_le_bytes([buf[pos + 9], buf[pos + 10]]) as usize;
            pos += 11;

            if pos + name_len > limit {
                break;
            }

            if let Ok(name) = core::str::from_utf8(&buf[pos..pos + name_len]) {
                if name != "." && name != ".." {
                    let mut path_buf = [0u8; MAX_PATH];
                    let mut o = 0;
                    let cbytes = current_dir.as_bytes();
                    if o + cbytes.len() < path_buf.len() {
                        path_buf[o..o + cbytes.len()].copy_from_slice(cbytes);
                        o += cbytes.len();
                    }
                    if o > 0 && path_buf[o - 1] != b'/' && o < path_buf.len() {
                        path_buf[o] = b'/';
                        o += 1;
                    }
                    let nbytes = name.as_bytes();
                    if o + nbytes.len() < path_buf.len() {
                        path_buf[o..o + nbytes.len()].copy_from_slice(nbytes);
                        o += nbytes.len();
                    }

                    if let Ok(full_path) = core::str::from_utf8(&path_buf[..o]) {
                        if pattern_matches(name, pattern) {
                            println!("{}", full_path);
                        }
                        if kind == 2 {
                            push_queue(full_path);
                        }
                    }
                }
            }
            pos += name_len;
        }
    }

    0
}
