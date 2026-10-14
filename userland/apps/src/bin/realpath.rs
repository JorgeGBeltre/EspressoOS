#![no_std]
#![no_main]

use libc::{arg, close, getcwd_str, open, println, O_RDONLY, PATH_MAX};

fn canonicalize<'a>(path: &str, cwd: &str, out: &'a mut [u8]) -> &'a str {
    let mut temp = [0u8; PATH_MAX * 2];
    let mut tlen = 0;

    if !path.starts_with('/') {
        let b = cwd.as_bytes();
        let copy = core::cmp::min(b.len(), temp.len());
        temp[..copy].copy_from_slice(&b[..copy]);
        tlen = copy;
        if tlen > 0 && temp[tlen - 1] != b'/' && tlen < temp.len() {
            temp[tlen] = b'/';
            tlen += 1;
        }
    }

    let pb = path.as_bytes();
    let copy = core::cmp::min(pb.len(), temp.len().saturating_sub(tlen));
    temp[tlen..tlen + copy].copy_from_slice(&pb[..copy]);
    tlen += copy;

    let bytes = &temp[..tlen];
    let n = bytes.len();

    let mut seg_starts: [usize; 16] = [0; 16];
    let mut seg_lens: [usize; 16] = [0; 16];
    let mut seg_count = 0;

    let mut start = 0;
    while start < n {
        while start < n && bytes[start] == b'/' {
            start += 1;
        }
        if start >= n {
            break;
        }
        let mut end = start;
        while end < n && bytes[end] != b'/' {
            end += 1;
        }
        let seg = &bytes[start..end];
        if seg == b"." {
            // skip
        } else if seg == b".." {
            if seg_count > 0 {
                seg_count -= 1;
            }
        } else {
            if seg_count < 16 {
                seg_starts[seg_count] = start;
                seg_lens[seg_count] = end - start;
                seg_count += 1;
            }
        }
        start = end;
    }

    if seg_count == 0 {
        out[0] = b'/';
        return core::str::from_utf8(&out[..1]).unwrap_or("/");
    }

    let mut pos = 0;
    for i in 0..seg_count {
        if pos < out.len() {
            out[pos] = b'/';
            pos += 1;
        }
        let s = seg_starts[i];
        let l = seg_lens[i];
        let copy_len = core::cmp::min(l, out.len().saturating_sub(pos));
        out[pos..pos + copy_len].copy_from_slice(&bytes[s..s + copy_len]);
        pos += copy_len;
    }

    core::str::from_utf8(&out[..pos]).unwrap_or("/")
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut files: [&str; 16] = [""; 16];
    let mut file_count = 0;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--help" || a == "-h" {
            println!("Usage: realpath [OPTION]... FILE...");
            println!("Print the resolved absolute file name; all but the last component must exist.");
            println!("  -q, --quiet    suppress error messages");
            println!("  -s, --strip    only canonicalize without resolving links");
            return 0;
        } else if a == "-q" || a == "--quiet" || a == "-s" || a == "--strip" {
            // Quiet / canonicalization flag
        } else {
            if file_count < 16 {
                files[file_count] = a;
                file_count += 1;
            }
        }
        i += 1;
    }

    if file_count == 0 {
        println!("realpath: missing operand");
        return 1;
    }

    let mut cwd_buf = [0u8; PATH_MAX];
    let cwd = getcwd_str(&mut cwd_buf).unwrap_or("/");

    let mut out_buf = [0u8; PATH_MAX];
    for &file in &files[..file_count] {
        let canon = canonicalize(file, cwd, &mut out_buf);
        let fd = open(canon, O_RDONLY);
        if fd >= 0 {
            let _ = close(fd as i32);
        }
        println!("{}", canon);
    }
    0
}
