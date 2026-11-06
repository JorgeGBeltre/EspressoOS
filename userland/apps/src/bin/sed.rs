#![no_std]
#![no_main]

use libc::{
    arg, close, open, print, println, read, rename, unlink, write, O_CREATE, O_RDONLY, O_TRUNC,
    O_WRONLY, PATH_MAX,
};

#[derive(Clone, Copy)]
enum Addr<'a> {
    Any,
    Line(usize),
    Last,
    Pat(&'a str),
}

#[derive(Clone, Copy)]
enum Action<'a> {
    Delete,
    Print,
    Subst {
        find: &'a str,
        rep: &'a str,
        global: bool,
        print_on_match: bool,
    },
}

#[derive(Clone, Copy)]
struct Cmd<'a> {
    addr: Addr<'a>,
    act: Action<'a>,
}

fn parse_usize(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let mut v = 0usize;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            v = v.saturating_mul(10).saturating_add((b - b'0') as usize);
        } else {
            return None;
        }
    }
    Some(v)
}

fn parse_cmd<'a>(raw: &'a str) -> Option<Cmd<'a>> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let (addr, rest) = if bytes[0] == b'/' {
        let mut end = None;
        for (i, &b) in bytes[1..].iter().enumerate() {
            if b == b'/' {
                end = Some(i + 1);
                break;
            }
        }
        let e = end?;
        (Addr::Pat(&s[1..e]), s[e + 1..].trim_start())
    } else if bytes[0] == b'$' {
        (Addr::Last, s[1..].trim_start())
    } else if bytes[0] >= b'0' && bytes[0] <= b'9' {
        let mut k = 0;
        while k < bytes.len() && bytes[k] >= b'0' && bytes[k] <= b'9' {
            k += 1;
        }
        (Addr::Line(parse_usize(&s[..k])?), s[k..].trim_start())
    } else {
        (Addr::Any, s)
    };

    if rest == "d" {
        return Some(Cmd { addr, act: Action::Delete });
    }
    if rest == "p" {
        return Some(Cmd { addr, act: Action::Print });
    }
    if rest.starts_with('s') && rest.len() >= 4 {
        let rb = rest.as_bytes();
        let delim = rb[1];
        let after_s = &rest[2..];
        let p1 = after_s.as_bytes().iter().position(|&b| b == delim)?;
        let find = &after_s[..p1];
        let after_f = &after_s[p1 + 1..];
        let p2 = after_f.as_bytes().iter().position(|&b| b == delim)?;
        let rep = &after_f[..p2];
        let flags = &after_f[p2 + 1..];
        let global = flags.contains('g');
        let print_on_match = flags.contains('p');
        return Some(Cmd {
            addr,
            act: Action::Subst {
                find,
                rep,
                global,
                print_on_match,
            },
        });
    }
    None
}

fn addr_matches(addr: Addr<'_>, line: &str, line_no: usize, is_last: bool) -> bool {
    match addr {
        Addr::Any => true,
        Addr::Line(n) => line_no == n,
        Addr::Last => is_last,
        Addr::Pat(p) => p.is_empty() || line.contains(p),
    }
}

fn subst_into(line: &str, find: &str, rep: &str, global: bool, out: &mut [u8; 512]) -> (usize, bool) {
    let lb = line.as_bytes();
    let fb = find.as_bytes();
    let rb = rep.as_bytes();
    if fb.is_empty() {
        let c = lb.len().min(out.len());
        out[..c].copy_from_slice(&lb[..c]);
        return (c, false);
    }
    let mut cursor = 0usize;
    let mut opos = 0usize;
    let mut matched = false;

    while cursor < lb.len() {
        if cursor + fb.len() <= lb.len() && &lb[cursor..cursor + fb.len()] == fb {
            let c = rb.len().min(out.len().saturating_sub(opos));
            out[opos..opos + c].copy_from_slice(&rb[..c]);
            opos += c;
            cursor += fb.len();
            matched = true;
            if !global {
                let rem = lb.len() - cursor;
                let rc = rem.min(out.len().saturating_sub(opos));
                out[opos..opos + rc].copy_from_slice(&lb[cursor..cursor + rc]);
                opos += rc;
                return (opos, true);
            }
        } else {
            if opos < out.len() {
                out[opos] = lb[cursor];
                opos += 1;
            }
            cursor += 1;
        }
    }
    (opos, matched)
}

fn emit_line(out_fd: i32, bytes: &[u8]) {
    if out_fd == 1 {
        if let Ok(s) = core::str::from_utf8(bytes) {
            println!("{}", s);
        }
    } else {
        let _ = write(out_fd, bytes);
        let _ = write(out_fd, b"\n");
    }
}

fn process_line(
    raw_line: &[u8],
    line_no: usize,
    is_last: bool,
    cmds: &[Cmd<'_>],
    quiet: bool,
    out_fd: i32,
) {
    let mut cur = [0u8; 512];
    let mut clen = raw_line.len().min(cur.len());
    cur[..clen].copy_from_slice(&raw_line[..clen]);

    for c in cmds {
        let s = match core::str::from_utf8(&cur[..clen]) {
            Ok(v) => v,
            Err(_) => return,
        };
        if !addr_matches(c.addr, s, line_no, is_last) {
            continue;
        }
        match c.act {
            Action::Delete => return,
            Action::Print => emit_line(out_fd, &cur[..clen]),
            Action::Subst {
                find,
                rep,
                global,
                print_on_match,
            } => {
                let mut next = [0u8; 512];
                let (nlen, matched) = subst_into(s, find, rep, global, &mut next);
                cur[..nlen].copy_from_slice(&next[..nlen]);
                clen = nlen;
                if matched && print_on_match {
                    emit_line(out_fd, &cur[..clen]);
                }
            }
        }
    }

    if !quiet {
        emit_line(out_fd, &cur[..clen]);
    }
}

fn run_stream(in_fd: i32, out_fd: i32, cmds: &[Cmd<'_>], quiet: bool) {
    let mut rbuf = [0u8; 256];
    let mut line_buf = [0u8; 512];
    let mut line_len = 0usize;
    let mut pending = [0u8; 512];
    let mut pending_len = 0usize;
    let mut has_pending = false;
    let mut line_no = 0usize;

    loop {
        let n = read(in_fd, &mut rbuf);
        if n <= 0 {
            break;
        }
        for &b in &rbuf[..n as usize] {
            if b == b'\n' {
                if line_len > 0 && line_buf[line_len - 1] == b'\r' {
                    line_len -= 1;
                }
                if has_pending {
                    line_no += 1;
                    process_line(&pending[..pending_len], line_no, false, cmds, quiet, out_fd);
                }
                pending[..line_len].copy_from_slice(&line_buf[..line_len]);
                pending_len = line_len;
                has_pending = true;
                line_len = 0;
            } else if line_len < line_buf.len() {
                line_buf[line_len] = b;
                line_len += 1;
            }
        }
    }

    if line_len > 0 {
        if line_buf[line_len - 1] == b'\r' {
            line_len -= 1;
        }
        if has_pending {
            line_no += 1;
            process_line(&pending[..pending_len], line_no, false, cmds, quiet, out_fd);
        }
        line_no += 1;
        process_line(&line_buf[..line_len], line_no, true, cmds, quiet, out_fd);
    } else if has_pending {
        line_no += 1;
        process_line(&pending[..pending_len], line_no, true, cmds, quiet, out_fd);
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: sed [-n] [-i] [-e script] [script] [file...]");
        return 1;
    }

    let mut quiet = false;
    let mut inplace = false;
    let mut exprs = [""; 8];
    let mut ecount = 0usize;
    let mut files = [""; 8];
    let mut fcount = 0usize;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("usage: sed [-n] [-i] [-e script] [script] [file...]");
            return 0;
        } else if a == "-n" || a == "--quiet" || a == "--silent" {
            quiet = true;
        } else if a == "-i" {
            inplace = true;
        } else if a == "-e" && i + 1 < argc {
            i += 1;
            if ecount < exprs.len() {
                exprs[ecount] = unsafe { arg(argv, i) };
                ecount += 1;
            }
        } else if ecount == 0 && !a.starts_with('-') {
            exprs[0] = a;
            ecount = 1;
        } else if fcount < files.len() {
            files[fcount] = a;
            fcount += 1;
        }
        i += 1;
    }

    if ecount == 0 {
        println!("sed: missing script expression");
        return 1;
    }

    let dummy = Cmd { addr: Addr::Any, act: Action::Delete };
    let mut cmds = [dummy; 12];
    let mut ccount = 0usize;
    for k in 0..ecount {
        for part in exprs[k].split(';') {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            match parse_cmd(trimmed) {
                Some(c) if ccount < cmds.len() => {
                    cmds[ccount] = c;
                    ccount += 1;
                }
                _ => {
                    print!("sed: unsupported expression '");
                    print!("{}", trimmed);
                    println!("'");
                    return 1;
                }
            }
        }
    }

    if fcount == 0 {
        if inplace {
            println!("sed: -i requires at least one file operand");
            return 1;
        }
        run_stream(0, 1, &cmds[..ccount], quiet);
        return 0;
    }

    let mut status = 0;
    for k in 0..fcount {
        let path = files[k];
        let in_fd = open(path, O_RDONLY);
        if in_fd < 0 {
            println!("sed: cannot open '{}'", path);
            status = 1;
            continue;
        }
        if !inplace {
            run_stream(in_fd as i32, 1, &cmds[..ccount], quiet);
            close(in_fd as i32);
        } else {
            let mut tbuf = [0u8; PATH_MAX];
            let pb = path.as_bytes();
            let suffix = b".sedtmp";
            if pb.len() + suffix.len() >= tbuf.len() {
                close(in_fd as i32);
                println!("sed: path too long '{}'", path);
                status = 1;
                continue;
            }
            tbuf[..pb.len()].copy_from_slice(pb);
            tbuf[pb.len()..pb.len() + suffix.len()].copy_from_slice(suffix);
            let tmp_path = core::str::from_utf8(&tbuf[..pb.len() + suffix.len()]).unwrap_or("");
            let out_fd = open(tmp_path, O_WRONLY | O_CREATE | O_TRUNC);
            if out_fd < 0 {
                close(in_fd as i32);
                println!("sed: cannot create temporary file for '{}'", path);
                status = 1;
                continue;
            }
            run_stream(in_fd as i32, out_fd as i32, &cmds[..ccount], quiet);
            close(in_fd as i32);
            close(out_fd as i32);
            if rename(tmp_path, path) < 0 {
                let _ = unlink(tmp_path);
                println!("sed: failed to replace '{}'", path);
                status = 1;
            }
        }
    }
    status
}
