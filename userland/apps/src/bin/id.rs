#![no_std]
#![no_main]

use libc::{arg, close, getuid_gid, open, println, read, O_RDONLY};

fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut v = 0u32;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            v = v.saturating_mul(10).saturating_add((b - b'0') as u32);
        } else {
            return None;
        }
    }
    Some(v)
}

fn resolve_user_by_uid<'a>(uid: u32, pbuf: &'a [u8]) -> Option<(&'a str, u32)> {
    let text = core::str::from_utf8(pbuf).ok()?;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let mut p = t.split(':');
        let u = p.next().unwrap_or("");
        let _pw = p.next();
        if let Some(us) = p.next() {
            if parse_u32(us) == Some(uid) {
                let g = p.next().and_then(parse_u32).unwrap_or(uid);
                return Some((u, g));
            }
        } else if uid == 0 && !u.is_empty() {
            return Some((u, 0));
        }
    }
    None
}

fn resolve_user_by_name<'a>(name: &'a str, pbuf: &'a [u8]) -> Option<(u32, u32)> {
    if let Ok(text) = core::str::from_utf8(pbuf) {
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let mut p = t.split(':');
            let u = p.next().unwrap_or("");
            let _pw = p.next();
            if u == name {
                let uid = p.next().and_then(parse_u32).unwrap_or(0);
                let gid = p.next().and_then(parse_u32).unwrap_or(uid);
                return Some((uid, gid));
            }
        }
    }
    if name == "root" {
        Some((0, 0))
    } else {
        None
    }
}

fn resolve_group_by_gid<'a>(gid: u32, gbuf: &'a [u8]) -> &'a str {
    if let Ok(text) = core::str::from_utf8(gbuf) {
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let mut p = t.split(':');
            let g = p.next().unwrap_or("");
            let _pw = p.next();
            if p.next().and_then(parse_u32) == Some(gid) && !g.is_empty() {
                return g;
            }
        }
    }
    match gid {
        0 => "root",
        10 => "wheel",
        100 => "users",
        _ => "users",
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut print_u_only = false;
    let mut print_g_only = false;
    let mut print_name_only = false;
    let mut target_user = "";

    let mut i = 1;
    while i < argc {
        let flag = unsafe { arg(argv, i) };
        if flag == "-u" {
            print_u_only = true;
        } else if flag == "-g" {
            print_g_only = true;
        } else if flag == "-n" {
            print_name_only = true;
        } else if flag == "-un" || flag == "-nu" {
            print_u_only = true;
            print_name_only = true;
        } else if flag == "-gn" || flag == "-ng" {
            print_g_only = true;
            print_name_only = true;
        } else if !flag.starts_with('-') {
            target_user = flag;
        }
        i += 1;
    }

    let mut pbuf = [0u8; 512];
    let mut plen = 0usize;
    let pfd = open("/etc/passwd", O_RDONLY);
    if pfd >= 0 {
        let n = read(pfd as i32, &mut pbuf);
        close(pfd as i32);
        if n > 0 {
            plen = n as usize;
        }
    }

    let mut gbuf = [0u8; 512];
    let mut glen = 0usize;
    let gfd = open("/etc/group", O_RDONLY);
    if gfd >= 0 {
        let n = read(gfd as i32, &mut gbuf);
        close(gfd as i32);
        if n > 0 {
            glen = n as usize;
        }
    }

    let (uid, gid, uname) = if !target_user.is_empty() {
        match resolve_user_by_name(target_user, &pbuf[..plen]) {
            Some((u, g)) => (u, g, target_user),
            None => {
                println!("id: '{}': no such user", target_user);
                return 1;
            }
        }
    } else {
        let (ku, kg) = getuid_gid();
        match resolve_user_by_uid(ku, &pbuf[..plen]) {
            Some((name, _)) => (ku, kg, name),
            None => (ku, kg, if ku == 0 { "root" } else { "user" }),
        }
    };

    let gname = resolve_group_by_gid(gid, &gbuf[..glen]);

    if print_u_only {
        if print_name_only {
            println!("{}", uname);
        } else {
            println!("{}", uid);
        }
        return 0;
    }

    if print_g_only {
        if print_name_only {
            println!("{}", gname);
        } else {
            println!("{}", gid);
        }
        return 0;
    }

    println!(
        "uid={}({}) gid={}({}) groups={}({})",
        uid, uname, gid, gname, gid, gname
    );
    0
}
