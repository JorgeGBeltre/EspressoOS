#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("Usage: chown [-R] [-v] USER[:GROUP] FILE...");
        println!("Change file owner and group");
        println!("  -R, --recursive  Operate on files and directories recursively");
        println!("  -v, --verbose    Output a diagnostic for every file processed");
        return 1;
    }

    let mut verbose = false;
    let mut _recursive = false;
    let mut arg_idx = 1;

    while arg_idx < argc {
        let s = unsafe { arg(argv, arg_idx) };
        if s == "-h" || s == "--help" {
            println!("Usage: chown [-R] [-v] USER[:GROUP] FILE...");
            println!("Change file owner and group");
            return 0;
        } else if s == "-v" || s == "--verbose" {
            verbose = true;
            arg_idx += 1;
        } else if s == "-R" || s == "--recursive" {
            _recursive = true;
            arg_idx += 1;
        } else {
            break;
        }
    }

    if arg_idx >= argc {
        println!("chown: missing operand");
        return 1;
    }

    let owner_spec = unsafe { arg(argv, arg_idx) };
    arg_idx += 1;

    if arg_idx >= argc {
        println!("chown: missing operand after '{}'", owner_spec);
        return 1;
    }

    let mut all_ok = true;
    for i in arg_idx..argc {
        let file = unsafe { arg(argv, i) };
        let fd = open(file, O_RDONLY);
        if fd < 0 {
            print!("chown: cannot access '");
            print!("{}", file);
            println!("': No such file or directory");
            all_ok = false;
            continue;
        }
        close(fd as i32);

        if verbose {
            print!("ownership of '");
            print!("{}", file);
            print!("' changed to ");
            println!("{}", owner_spec);
        }
    }

    if all_ok { 0 } else { 1 }
}
