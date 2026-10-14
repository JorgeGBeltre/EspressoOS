#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, seek, write, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY};

struct StderrWriter;
impl core::fmt::Write for StderrWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let _ = write(2, s.as_bytes());
        Ok(())
    }
}

fn print_err(args: core::fmt::Arguments) {
    let mut w = StderrWriter;
    let _ = core::fmt::Write::write_fmt(&mut w, args);
}

macro_rules! eprintln {
    ($($arg:tt)*) => {{
        print_err(format_args!($($arg)*));
        print_err(format_args!("\n"));
    }};
}

fn parse_num(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let last = bytes[bytes.len() - 1];
    let (num_str, mult) = match last {
        b'c' | b'C' => (&s[..s.len() - 1], 1usize),
        b'w' | b'W' => (&s[..s.len() - 1], 2usize),
        b'b' | b'B' => (&s[..s.len() - 1], 512usize),
        b'k' | b'K' => (&s[..s.len() - 1], 1024usize),
        b'm' | b'M' => (&s[..s.len() - 1], 1024 * 1024usize),
        b'0'..=b'9' => (s, 1usize),
        _ => return None,
    };
    let val = num_str.parse::<usize>().ok()?;
    val.checked_mul(mult)
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut if_path: Option<&str> = None;
    let mut of_path: Option<&str> = None;
    let mut bs: usize = 512;
    let mut ibs: Option<usize> = None;
    let mut obs: Option<usize> = None;
    let mut count: Option<usize> = None;
    let mut skip: usize = 0;
    let mut seek_blocks: usize = 0;
    let mut status_none = false;
    let mut notrunc = false;

    let mut i = 1;
    while i < argc {
        let s = unsafe { arg(argv, i) };
        if s == "-h" || s == "--help" {
            println!("Usage: dd [OPERAND]...");
            println!("Copy a file, converting and formatting according to the operands.");
            println!("  bs=BYTES      Read and write up to BYTES bytes at a time (default: 512)");
            println!("  ibs=BYTES     Read up to BYTES bytes at a time");
            println!("  obs=BYTES     Write up to BYTES bytes at a time");
            println!("  count=N       Copy only N input blocks");
            println!("  skip=N        Skip N ibs-sized blocks at start of input");
            println!("  seek=N        Skip N obs-sized blocks at start of output");
            println!("  if=FILE       Read from FILE instead of stdin");
            println!("  of=FILE       Write to FILE instead of stdout");
            println!("  conv=notrunc  Do not truncate the output file");
            println!("  status=none   Suppress transfer statistics");
            return 0;
        }

        if let Some(rest) = s.strip_prefix("if=") {
            if_path = Some(rest);
        } else if let Some(rest) = s.strip_prefix("of=") {
            of_path = Some(rest);
        } else if let Some(rest) = s.strip_prefix("bs=") {
            if let Some(v) = parse_num(rest) {
                if v > 0 {
                    bs = v;
                }
            } else {
                eprintln!("dd: invalid number: '{}'", rest);
                return 1;
            }
        } else if let Some(rest) = s.strip_prefix("ibs=") {
            if let Some(v) = parse_num(rest) {
                if v > 0 {
                    ibs = Some(v);
                }
            }
        } else if let Some(rest) = s.strip_prefix("obs=") {
            if let Some(v) = parse_num(rest) {
                if v > 0 {
                    obs = Some(v);
                }
            }
        } else if let Some(rest) = s.strip_prefix("count=") {
            count = parse_num(rest);
        } else if let Some(rest) = s.strip_prefix("skip=") {
            if let Some(v) = parse_num(rest) {
                skip = v;
            }
        } else if let Some(rest) = s.strip_prefix("seek=") {
            if let Some(v) = parse_num(rest) {
                seek_blocks = v;
            }
        } else if s == "conv=notrunc" {
            notrunc = true;
        } else if s == "status=none" {
            status_none = true;
        }

        i += 1;
    }

    let in_bs = ibs.unwrap_or(bs);
    let out_bs = obs.unwrap_or(bs);

    let if_fd = match if_path {
        Some(p) if p != "-" => {
            let fd = open(p, O_RDONLY);
            if fd < 0 {
                eprintln!("dd: failed to open '{}' for reading", p);
                return 1;
            }
            fd as i32
        }
        _ => 0, // stdin
    };

    let of_fd = match of_path {
        Some(p) if p != "-" => {
            let mut flags = O_WRONLY | O_CREATE;
            if !notrunc && seek_blocks == 0 {
                flags |= O_TRUNC;
            }
            let fd = open(p, flags);
            if fd < 0 {
                eprintln!("dd: failed to open '{}' for writing", p);
                if if_fd > 0 {
                    close(if_fd);
                }
                return 1;
            }
            fd as i32
        }
        _ => 1, // stdout
    };

    let mut buf = [0u8; 1024];

    // Handle skip
    if skip > 0 {
        let skip_bytes = skip.saturating_mul(in_bs);
        let seek_res = seek(if_fd, skip_bytes as isize, 0);
        if seek_res < 0 {
            let mut rem = skip_bytes;
            while rem > 0 {
                let to_read = rem.min(buf.len());
                let n = read(if_fd, &mut buf[..to_read]);
                if n <= 0 {
                    break;
                }
                rem -= n as usize;
            }
        }
    }

    // Handle seek
    if seek_blocks > 0 {
        let seek_bytes = seek_blocks.saturating_mul(out_bs);
        let _ = seek(of_fd, seek_bytes as isize, 0);
    }

    let mut in_full: usize = 0;
    let mut in_partial: usize = 0;
    let mut out_full: usize = 0;
    let mut out_partial: usize = 0;
    let mut total_bytes: usize = 0;
    let mut blocks_read: usize = 0;

    'copy_loop: loop {
        if let Some(max_cnt) = count {
            if blocks_read >= max_cnt {
                break 'copy_loop;
            }
        }

        // Read up to in_bs
        let mut cur_in = 0usize;
        while cur_in < in_bs {
            let want = (in_bs - cur_in).min(buf.len());
            let n = read(if_fd, &mut buf[..want]);
            if n < 0 {
                eprintln!("dd: read error");
                break 'copy_loop;
            }
            if n == 0 {
                break;
            }
            let cnt = n as usize;

            // Write to output
            let mut cur_out = 0usize;
            while cur_out < cnt {
                let wn = write(of_fd, &buf[cur_out..cnt]);
                if wn <= 0 {
                    eprintln!("dd: write error");
                    break 'copy_loop;
                }
                cur_out += wn as usize;
                total_bytes += wn as usize;
            }

            cur_in += cnt;
            if cnt < want {
                // Short read from input
                break;
            }
        }

        if cur_in == 0 {
            // EOF reached
            break;
        }

        blocks_read += 1;
        if cur_in == in_bs {
            in_full += 1;
        } else {
            in_partial += 1;
        }

        let full_out_recs = cur_in / out_bs;
        let rem_out_recs = cur_in % out_bs;
        out_full += full_out_recs;
        if rem_out_recs > 0 {
            out_partial += 1;
        }
    }

    if if_fd > 0 {
        close(if_fd);
    }
    if of_fd > 1 {
        close(of_fd);
    }

    if !status_none {
        eprintln!("{}+{} records in", in_full, in_partial);
        eprintln!("{}+{} records out", out_full, out_partial);
        eprintln!("{} bytes copied", total_bytes);
    }

    0
}
