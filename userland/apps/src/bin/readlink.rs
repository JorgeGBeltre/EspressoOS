#![no_std]
#![no_main]

use libc::{arg, close, getcwd_str, open, print, println, O_RDONLY, PATH_MAX};

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
    let mut canonical = false;
    let mut no_newline = false;
    let mut quiet = false;
    let mut target = "";

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--help" || a == "-h" {
            println!("Usage: readlink [OPTION]... FILE...");
            println!("Print value of a symbolic link or canonical file name");
            println!("  -f, --canonicalize  canonicalize by following every component");
            println!("  -n, --no-newline    do not output the trailing delimiter");
            println!("  -q, --quiet         suppress most error messages");
            return 0;
        } else if a == "-f" || a == "--canonicalize" {
            canonical = true;
        } else if a == "-n" || a == "--no-newline" {
            no_newline = true;
        } else if a == "-q" || a == "--quiet" {
            quiet = true;
        } else if !a.starts_with('-') || a == "-" {
            target = a;
        }
        i += 1;
    }

    if target.is_empty() {
        println!("Usage: readlink [OPTION]... FILE...");
        return 1;
    }

    let mut cwd_buf = [0u8; PATH_MAX];
    let cwd = getcwd_str(&mut cwd_buf).unwrap_or("/");

    let mut out_buf = [0u8; PATH_MAX * 2];
    let resolved = canonicalize(target, cwd, &mut out_buf);

    if canonical {
        let fd = open(resolved, O_RDONLY);
        if fd < 0 {
            if !quiet {
                println!("readlink: {}: No such file or directory", target);
            }
            return 1;
        }
        let _ = close(fd as i32);

        if no_newline {
            print!("{}", resolved);
        } else {
            println!("{}", resolved);
        }
    } else {
        if no_newline {
            print!("{}", target);
        } else {
            println!("{}", target);
        }
    }

    0
}
