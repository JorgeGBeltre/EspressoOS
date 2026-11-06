#![no_std]
#![no_main]

use libc::{arg, print, println, symlink, unlink};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("Usage: ln [-s] [-f] [-v] TARGET LINK_NAME");
        return 1;
    }

    let mut symbolic = false;
    let mut force = false;
    let mut verbose = false;
    let mut arg_idx = 1;

    while arg_idx < argc {
        let s = unsafe { arg(argv, arg_idx) };
        if s == "-h" || s == "--help" {
            println!("Usage: ln [-s] [-f] [-v] TARGET LINK_NAME");
            println!("  -s, --symbolic  Make symbolic links instead of hard links");
            println!("  -f, --force     Remove existing destination files");
            println!("  -v, --verbose   Print name of each linked file");
            return 0;
        } else if s.starts_with('-') && s.len() > 1 && !s.starts_with("--") {
            let mut recognized = true;
            for &b in &s.as_bytes()[1..] {
                match b {
                    b's' => symbolic = true,
                    b'f' => force = true,
                    b'v' => verbose = true,
                    b'n' => {}
                    _ => recognized = false,
                }
            }
            if recognized {
                arg_idx += 1;
            } else {
                break;
            }
        } else if s == "--symbolic" {
            symbolic = true;
            arg_idx += 1;
        } else if s == "--force" {
            force = true;
            arg_idx += 1;
        } else if s == "--verbose" {
            verbose = true;
            arg_idx += 1;
        } else {
            break;
        }
    }

    if arg_idx + 1 >= argc {
        println!("ln: missing destination file operand");
        return 1;
    }

    let target = unsafe { arg(argv, arg_idx) };
    let link_name = unsafe { arg(argv, arg_idx + 1) };

    if !symbolic {
        print!("ln: failed to create hard link '");
        print!("{}", link_name);
        print!("' => '");
        print!("{}", target);
        println!("': Operation not supported (use ln -s)");
        return 1;
    }

    if force {
        let _ = unlink(link_name);
    }

    let rc = symlink(target, link_name);
    if rc < 0 {
        let msg = match rc {
            -17 => "File exists",
            -2 => "No such file or directory",
            -1 => "Operation not permitted",
            _ => "Cannot create symlink",
        };
        println!("ln: failed to create symbolic link '{}': {}", link_name, msg);
        return 1;
    }

    if verbose {
        println!("'{}' -> '{}'", link_name, target);
    }
    0
}
