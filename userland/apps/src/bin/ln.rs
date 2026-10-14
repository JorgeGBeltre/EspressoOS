#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, unlink, write, O_CREATE, O_TRUNC, O_WRONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("Usage: ln [-s] [-f] [-v] TARGET LINK_NAME");
        println!("Make links between files");
        println!("  -s, --symbolic  Make symbolic links instead of hard links");
        println!("  -f, --force     Remove existing destination files");
        println!("  -v, --verbose   Print name of each linked file");
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
            println!("Make links between files");
            println!("  -s, --symbolic  Make symbolic links instead of hard links");
            println!("  -f, --force     Remove existing destination files");
            println!("  -v, --verbose   Print name of each linked file");
            return 0;
        } else if s == "-s" || s == "--symbolic" {
            symbolic = true;
            arg_idx += 1;
        } else if s == "-f" || s == "--force" {
            force = true;
            arg_idx += 1;
        } else if s == "-v" || s == "--verbose" {
            verbose = true;
            arg_idx += 1;
        } else if s == "-sf" || s == "-fs" {
            symbolic = true;
            force = true;
            arg_idx += 1;
        } else {
            break;
        }
    }

    if arg_idx + 1 >= argc {
        println!("ln: missing destination file operand after '{}'", unsafe { arg(argv, arg_idx) });
        return 1;
    }

    let target = unsafe { arg(argv, arg_idx) };
    let link_name = unsafe { arg(argv, arg_idx + 1) };

    if force {
        let _ = unlink(link_name);
    }

    if symbolic {
        // Write the target path into the link file (symbolic link representation)
        let fd = open(link_name, O_CREATE | O_WRONLY | O_TRUNC);
        if fd < 0 {
            print!("ln: failed to create symbolic link '");
            print!("{}", link_name);
            println!("'");
            return 1;
        }
        let _ = write(fd as i32, target.as_bytes());
        close(fd as i32);

        if verbose {
            print!("'");
            print!("{}", link_name);
            print!("' -> '");
            print!("{}", target);
            println!("'");
        }
        0
    } else {
        // Hard link on this filesystem
        print!("ln: failed to create hard link '");
        print!("{}", link_name);
        print!("' => '");
        print!("{}", target);
        println!("': Operation not supported");
        1
    }
}
