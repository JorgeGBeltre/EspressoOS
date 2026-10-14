#![no_std]
#![no_main]

use libc::{arg, println, read, spawn, wait};

const MAX_ARGS: usize = 16;
const TOKEN_BUF_LEN: usize = 64;

struct TokenBuf {
    buf: [u8; TOKEN_BUF_LEN],
    len: usize,
}

impl TokenBuf {
    const fn new() -> Self {
        Self {
            buf: [0u8; TOKEN_BUF_LEN],
            len: 0,
        }
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut cmd_name = "echo";
    let mut initial_args_start = 2;

    if argc >= 2 {
        let first = unsafe { arg(argv, 1) };
        if first == "-h" || first == "--help" {
            println!("usage: xargs [COMMAND [INITIAL-ARGS...]]");
            println!("Build and execute command lines from standard input");
            return 0;
        }
        cmd_name = first;
    } else {
        initial_args_start = 1;
    }

    // Prepare path: e.g. "/bin/echo" or path as-is if starts with '/'
    let mut path_buf = [0u8; 64];
    let mut p_idx = 0;
    if !cmd_name.starts_with('/') && !cmd_name.starts_with('.') {
        for &b in b"/bin/" {
            path_buf[p_idx] = b;
            p_idx += 1;
        }
    }
    for &b in cmd_name.as_bytes() {
        if p_idx < path_buf.len() - 1 {
            path_buf[p_idx] = b;
            p_idx += 1;
        }
    }
    let full_path = core::str::from_utf8(&path_buf[..p_idx]).unwrap_or("/bin/echo");

    // Static buffers for tokens from stdin
    let mut token_storage: [TokenBuf; MAX_ARGS] = [const { TokenBuf::new() }; MAX_ARGS];
    let mut num_tokens = 0usize;

    let mut chunk = [0u8; 128];
    let mut cur_token = TokenBuf::new();

    loop {
        let n = read(0, &mut chunk);
        if n <= 0 {
            break;
        }
        for &byte in &chunk[..n as usize] {
            if byte == b' ' || byte == b'\t' || byte == b'\r' || byte == b'\n' {
                if cur_token.len > 0 && num_tokens < MAX_ARGS {
                    cur_token.buf[cur_token.len] = 0; // NUL terminate
                    token_storage[num_tokens] = cur_token;
                    num_tokens += 1;
                    cur_token = TokenBuf::new();
                }
            } else if cur_token.len < TOKEN_BUF_LEN - 1 {
                cur_token.buf[cur_token.len] = byte;
                cur_token.len += 1;
            }
        }
    }

    if cur_token.len > 0 && num_tokens < MAX_ARGS {
        cur_token.buf[cur_token.len] = 0;
        token_storage[num_tokens] = cur_token;
        num_tokens += 1;
    }

    // Build argv array
    let mut child_argv: [*const u8; 32] = [core::ptr::null(); 32];
    let mut a_idx = 0;

    // argv[0] = cmd_name
    child_argv[a_idx] = full_path.as_ptr();
    a_idx += 1;

    // Initial args from xargs command line
    for i in initial_args_start..argc {
        if a_idx < 31 {
            child_argv[a_idx] = unsafe { *argv.add(i as usize) };
            a_idx += 1;
        }
    }

    // Args collected from stdin
    for i in 0..num_tokens {
        if a_idx < 31 {
            child_argv[a_idx] = token_storage[i].buf.as_ptr();
            a_idx += 1;
        }
    }

    child_argv[a_idx] = core::ptr::null();

    let pid = spawn(full_path, child_argv.as_ptr());
    if pid < 0 {
        println!("xargs: cannot spawn '{}'", full_path);
        return 127;
    }

    let mut status = 0;
    let _ = wait(&mut status);
    status
}
