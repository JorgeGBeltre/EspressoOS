#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

fn query_file(path: &str, key: Option<&str>, default_lines: Option<&[&str]>) -> i32 {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        if let Some(defs) = default_lines {
            return match_and_print_lines(defs, key);
        }
        print!("getent: cannot open ");
        println!("{}", path);
        return 2;
    }
    let fd = fd as i32;

    let mut buf = [0u8; 512];
    let mut line_buf = [0u8; 128];
    let mut line_len = 0usize;
    let mut found = false;

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                if line_len > 0 {
                    if check_line(&line_buf[..line_len], key) {
                        found = true;
                    }
                    line_len = 0;
                }
            } else if line_len < line_buf.len() {
                line_buf[line_len] = b;
                line_len += 1;
            }
        }
    }
    if line_len > 0 {
        if check_line(&line_buf[..line_len], key) {
            found = true;
        }
    }
    close(fd);

    if found || key.is_none() {
        0
    } else {
        2
    }
}

fn check_line(line: &[u8], key: Option<&str>) -> bool {
    let s = match core::str::from_utf8(line) {
        Ok(v) => v.trim(),
        Err(_) => return false,
    };
    if s.is_empty() || s.starts_with("#") {
        return false;
    }

    match key {
        None => {
            println!("{}", s);
            true
        }
        Some(k) => {
            // Check colon-separated or whitespace-separated fields
            let matches_colon = s.split(':').any(|field| field == k);
            let matches_space = s.split_ascii_whitespace().any(|field| field == k);
            if matches_colon || matches_space {
                println!("{}", s);
                true
            } else {
                false
            }
        }
    }
}

fn match_and_print_lines(lines: &[&str], key: Option<&str>) -> i32 {
    let mut found = false;
    for &line in lines {
        if check_line(line.as_bytes(), key) {
            found = true;
        }
    }
    if found || key.is_none() {
        0
    } else {
        2
    }
}

const DEFAULT_PASSWD: &[&str] = &[
    "root:x:0:0:root:/home/root:/bin/sh",
    "admin:x:1000:1000:admin:/home/admin:/bin/sh",
];

const DEFAULT_GROUP: &[&str] = &[
    "root:x:0:",
    "admin:x:1000:",
    "users:x:100:",
];

const DEFAULT_HOSTS: &[&str] = &[
    "127.0.0.1 localhost",
    "::1 localhost ip6-localhost ip6-loopback",
];

const DEFAULT_SERVICES: &[&str] = &[
    "ssh 22/tcp",
    "http 80/tcp",
    "domain 53/udp",
    "ntp 123/udp",
];

const DEFAULT_PROTOCOLS: &[&str] = &[
    "ip 0 IP",
    "icmp 1 ICMP",
    "tcp 6 TCP",
    "udp 17 UDP",
];

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: getent DATABASE [KEY ...]");
        println!("Get entries from administrative databases");
        println!("Supported databases: passwd, group, hosts, services, protocols");
        return 1;
    }

    let first = unsafe { arg(argv, 1) };
    if first == "-h" || first == "--help" {
        println!("Usage: getent DATABASE [KEY ...]");
        println!("Get entries from administrative databases");
        println!("Supported databases: passwd, group, hosts, services, protocols");
        return 0;
    }

    let db = first;
    let keys_start = 2;

    let run_db = |key: Option<&str>| -> i32 {
        match db {
            "passwd" => query_file("/etc/passwd", key, Some(DEFAULT_PASSWD)),
            "group" => query_file("/etc/group", key, Some(DEFAULT_GROUP)),
            "hosts" => query_file("/etc/hosts", key, Some(DEFAULT_HOSTS)),
            "services" => query_file("/etc/services", key, Some(DEFAULT_SERVICES)),
            "protocols" => query_file("/etc/protocols", key, Some(DEFAULT_PROTOCOLS)),
            _ => {
                print!("getent: Unknown database: ");
                println!("{}", db);
                1
            }
        }
    };

    if keys_start >= argc {
        run_db(None)
    } else {
        let mut exit_code = 0;
        for i in keys_start..argc {
            let key = unsafe { arg(argv, i) };
            let r = run_db(Some(key));
            if r != 0 {
                exit_code = r;
            }
        }
        exit_code
    }
}
