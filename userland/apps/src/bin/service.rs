#![no_std]
#![no_main]

use libc::{arg, println, spawn, waitpid};

fn is_action(s: &str) -> bool {
    matches!(
        s,
        "start" | "stop" | "restart" | "status" | "reload" | "enable" | "disable"
    )
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        let child = [b"systemctl\0".as_ptr(), b"list-units\0".as_ptr(), core::ptr::null()];
        let pid = spawn("/bin/systemctl", child.as_ptr());
        if pid < 0 {
            return 1;
        }
        let mut st = 0;
        let _ = waitpid(pid as i32, &mut st, 0);
        return st;
    }

    let a1 = unsafe { arg(argv, 1) };
    if a1 == "-h" || a1 == "--help" {
        println!("Usage: service <unit> <start|stop|restart|status|enable|disable>");
        println!("   or: service [list | --status-all | <action> <unit>]");
        return 0;
    }

    if a1 == "list" || a1 == "--status-all" || (a1 == "status" && argc == 2) {
        let child = [b"systemctl\0".as_ptr(), b"list-units\0".as_ptr(), core::ptr::null()];
        let pid = spawn("/bin/systemctl", child.as_ptr());
        if pid < 0 {
            return 1;
        }
        let mut st = 0;
        let _ = waitpid(pid as i32, &mut st, 0);
        return st;
    }

    if argc < 3 {
        println!("Usage: service <unit> <start|stop|restart|status|enable|disable>");
        return 1;
    }

    let a2 = unsafe { arg(argv, 2) };
    // Support both `service httpd start` (SysV) and `service start httpd` (EspressoOS legacy)
    let (action, unit) = if is_action(a1) && !is_action(a2) {
        (a1, a2)
    } else {
        (a2, a1)
    };

    let mut abuf = [0u8; 24];
    let mut ubuf = [0u8; 32];
    let al = action.len().min(23);
    let ul = unit.len().min(31);
    abuf[..al].copy_from_slice(&action.as_bytes()[..al]);
    abuf[al] = 0;
    ubuf[..ul].copy_from_slice(&unit.as_bytes()[..ul]);
    ubuf[ul] = 0;

    let child = [
        b"systemctl\0".as_ptr(),
        abuf.as_ptr(),
        ubuf.as_ptr(),
        core::ptr::null(),
    ];
    let pid = spawn("/bin/systemctl", child.as_ptr());
    if pid < 0 {
        println!("service: failed to invoke /bin/systemctl");
        return 1;
    }
    let mut st = 0;
    let _ = waitpid(pid as i32, &mut st, 0);
    st
}
