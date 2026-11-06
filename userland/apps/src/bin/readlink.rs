#![no_std]
#![no_main]

use libc::{arg, getcwd_str, print, println, readlink, stat, StatBuf, PATH_MAX};

fn normalize_abs<'a>(raw: &[u8], out: &'a mut [u8; PATH_MAX]) -> &'a str {
    let mut seg_starts = [0usize; 24];
    let mut seg_lens = [0usize; 24];
    let mut count = 0usize;
    let n = raw.len();
    let mut i = 0usize;
    while i < n {
        while i < n && raw[i] == b'/' {
            i += 1;
        }
        if i >= n {
            break;
        }
        let s = i;
        while i < n && raw[i] != b'/' {
            i += 1;
        }
        let seg = &raw[s..i];
        if seg == b"." {
            continue;
        } else if seg == b".." {
            if count > 0 {
                count -= 1;
            }
        } else if count < 24 {
            seg_starts[count] = s;
            seg_lens[count] = i - s;
            count += 1;
        }
    }
    if count == 0 {
        out[0] = b'/';
        return core::str::from_utf8(&out[..1]).unwrap_or("/");
    }
    let mut pos = 0usize;
    for k in 0..count {
        if pos < out.len() {
            out[pos] = b'/';
            pos += 1;
        }
        let l = seg_lens[k].min(out.len().saturating_sub(pos));
        out[pos..pos + l].copy_from_slice(&raw[seg_starts[k]..seg_starts[k] + l]);
        pos += l;
    }
    core::str::from_utf8(&out[..pos]).unwrap_or("/")
}

fn join_base(base: &str, rel: &str, tmp: &mut [u8; PATH_MAX * 2]) -> usize {
    let mut tlen = 0usize;
    if !rel.starts_with('/') {
        let bb = base.as_bytes();
        let c = bb.len().min(tmp.len());
        tmp[..c].copy_from_slice(&bb[..c]);
        tlen = c;
        if tlen == 0 || tmp[tlen - 1] != b'/' {
            if tlen < tmp.len() {
                tmp[tlen] = b'/';
                tlen += 1;
            }
        }
    }
    let rb = rel.as_bytes();
    let c = rb.len().min(tmp.len().saturating_sub(tlen));
    tmp[tlen..tlen + c].copy_from_slice(&rb[..c]);
    tlen + c
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
            println!("Usage: readlink [-f] [-n] [-q] FILE");
            return 0;
        } else if a == "-f" || a == "--canonicalize" {
            canonical = true;
        } else if a == "-n" || a == "--no-newline" {
            no_newline = true;
        } else if a == "-q" || a == "--quiet" {
            quiet = true;
        } else if !a.starts_with('-') {
            target = a;
        }
        i += 1;
    }

    if target.is_empty() {
        println!("Usage: readlink [-f] [-n] [-q] FILE");
        return 1;
    }

    if !canonical {
        let mut lbuf = [0u8; PATH_MAX];
        let n = readlink(target, &mut lbuf);
        if n < 0 {
            return 1;
        }
        if let Ok(s) = core::str::from_utf8(&lbuf[..n as usize]) {
            if no_newline {
                print!("{}", s);
            } else {
                println!("{}", s);
            }
            return 0;
        }
        return 1;
    }

    let mut cwd_buf = [0u8; PATH_MAX];
    let cwd = getcwd_str(&mut cwd_buf).unwrap_or("/");

    let mut tmp = [0u8; PATH_MAX * 2];
    let mut cur_buf = [0u8; PATH_MAX];
    let tlen = join_base(cwd, target, &mut tmp);
    let mut cur_len = {
        let s = normalize_abs(&tmp[..tlen], &mut cur_buf);
        s.len()
    };

    for _ in 0..8 {
        let cur_str = core::str::from_utf8(&cur_buf[..cur_len]).unwrap_or("/");
        let mut lbuf = [0u8; PATH_MAX];
        let n = readlink(cur_str, &mut lbuf);
        if n < 0 {
            break;
        }
        let link_str = match core::str::from_utf8(&lbuf[..n as usize]) {
            Ok(s) => s,
            Err(_) => break,
        };
        let parent = match cur_str.rfind('/') {
            Some(0) | None => "/",
            Some(idx) => &cur_str[..idx],
        };
        let jlen = join_base(parent, link_str, &mut tmp);
        let mut next_buf = [0u8; PATH_MAX];
        let norm = normalize_abs(&tmp[..jlen], &mut next_buf);
        cur_len = norm.len();
        cur_buf[..cur_len].copy_from_slice(norm.as_bytes());
    }

    let resolved = core::str::from_utf8(&cur_buf[..cur_len]).unwrap_or("/");
    let mut st = StatBuf::zeroed();
    if stat(resolved, &mut st) < 0 {
        if !quiet {
            println!("readlink: {}: No such file or directory", target);
        }
        return 1;
    }

    if no_newline {
        print!("{}", resolved);
    } else {
        println!("{}", resolved);
    }
    0
}
