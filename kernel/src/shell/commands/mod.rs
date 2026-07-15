#![allow(dead_code)]

use crate::arch::xtensa::timer;
use crate::drivers::uart;
use crate::mm;
use crate::prelude::*;
use crate::scheduler;
use crate::vfs::{self, DirEntry, InodeKind, OpenFlags};
use alloc::format;

use super::parser::Redirect;

pub const STDIN: vfs::Fd = 0;
pub const STDOUT: vfs::Fd = 1;
pub const STDERR: vfs::Fd = 2;

// The cwd lives on the Process, not here. It used to be one kernel-global
// Mutex<String>, which meant `cd` in an SSH session silently moved the serial
// console's cwd too, and a fresh session inherited the previous one's directory.
pub fn cwd_get() -> String {
    scheduler::process::cwd_get()
}

fn norm_abs(path: &str) -> String {
    let mut comps: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                comps.pop();
            }
            p => comps.push(p),
        }
    }
    let mut out = String::from("/");
    out.push_str(&comps.join("/"));
    out
}

fn resolve(path: &str) -> String {
    if path.starts_with('/') {
        norm_abs(path)
    } else {
        let mut base = cwd_get();
        if !base.ends_with('/') {
            base.push('/');
        }
        base.push_str(path);
        norm_abs(&base)
    }
}

/// Writes every byte or gives up, whichever comes first.
///
/// A channel accepts short writes and answers WouldBlock when its ring is full,
/// so the retry loop lives here -- outside the VFS, where yielding is legal
/// because no lock is held. IoError means the session is gone; there is nowhere
/// left to put the rest, so drop it rather than spin.
pub(crate) fn write_all(fd: vfs::Fd, bytes: &[u8]) {
    let mut done = 0usize;
    while done < bytes.len() {
        match vfs::write(fd, &bytes[done..]) {
            Ok(0) | Err(KError::WouldBlock) => scheduler::yield_now(),
            Ok(n) => done += n,
            // The session hung up. Nowhere to put the rest, and nowhere to
            // complain to either.
            Err(KError::IoError) => return,
            // Anything else is the sink failing under us -- a full filesystem on a
            // redirected stdout, say. Swallowing that would let `echo x > /config`
            // truncate the file, write nothing and still return a clean prompt, so
            // say it on stderr, which `>` never captures. The recursion is one deep
            // at most: the inner call is on STDERR.
            Err(e) => {
                if fd != STDERR {
                    write_all(STDERR, format!("shell: write failed ({:?})\n", e).as_bytes());
                }
                return;
            }
        }
    }
}

fn emit(bytes: &[u8]) {
    write_all(STDOUT, bytes);
}

fn emit_str(s: &str) {
    emit(s.as_bytes());
}

// Emits a bare \n, never \r\n: turning that into CRLF is the terminal's job
// (see SessionChannel::write). A redirected stdout is a plain file inode, which
// translates nothing, so `echo x > f` gets LF the way it should.
fn emit_line(s: &str) {
    emit(s.as_bytes());
    emit(b"\n");
}

// stderr, so `>` never captures it -- which is the whole reason fd 2 exists.
fn eprint_line(s: &str) {
    write_all(STDERR, s.as_bytes());
    write_all(STDERR, b"\n");
}

/// Points STDOUT at `redirect`'s file, returning the saved original so
/// `end_redirect` can put it back. This is the plain dup2 dance: the command
/// underneath writes fd 1 and never learns it moved.
pub fn begin_redirect(redirect: &Redirect) -> KResult<Option<vfs::Fd>> {
    let (path, extra) = match redirect {
        Redirect::None => return Ok(None),
        Redirect::Truncate(p) => (p, OpenFlags::TRUNC.0),
        Redirect::Append(p) => (p, OpenFlags::APPEND.0),
    };

    let flags = OpenFlags(OpenFlags::WRONLY.0 | OpenFlags::CREATE.0 | extra);
    let fd = vfs::open(&resolve(path), flags)?;

    let saved = match vfs::dup(STDOUT) {
        Ok(s) => s,
        Err(e) => {
            let _ = vfs::close(fd);
            return Err(e);
        }
    };
    if let Err(e) = vfs::dup2(fd, STDOUT) {
        let _ = vfs::close(fd);
        let _ = vfs::close(saved);
        return Err(e);
    }
    let _ = vfs::close(fd);
    Ok(Some(saved))
}

pub fn end_redirect(saved: Option<vfs::Fd>) {
    if let Some(s) = saved {
        let _ = vfs::dup2(s, STDOUT);
        let _ = vfs::close(s);
    }
}

/// Priority the scheduler ignores -- `next_ready` only looks at affinity, and
/// `Task::priority` has no readers. Matches sys_spawn so the two agree if it ever
/// grows any.
const EXEC_PRIO: u8 = 10;

/// Runs a program from `/bin` (or a path, if the name has a slash) and waits.
///
/// No argv: `_args` is dropped on the floor. There is nowhere to put it -- `_start`
/// is `call4 main` with nothing marshalled, and every program's `main()` takes no
/// parameters. Wiring that up is its own job; this exists to answer the question
/// that cannot be answered by reading code, which is whether a child's output
/// reaches the session that launched it.
fn try_exec(name: &str, args: &[&str]) -> i32 {
    match spawn_child(name, args, STDIN, STDOUT, &[]) {
        Ok(pid) => wait_all(&[pid]),
        Err(code) => code,
    }
}

/// Writes the argv blob into the top of the child's data slot and returns a pointer
/// to it, which is what the task entry receives.
///
/// Layout, in the child's own addresses:
///
///     [argc: u32][argv[0]]..[argv[argc-1]][NULL][name\0][arg\0]..
///
/// argc leads so `_start` can unpack both of main's parameters from the single
/// usize the kernel passes: argc is `*blob` and argv is `blob + 4`.
///
/// It goes at the TOP of the slot growing down, not after .bss, so that its address
/// does not move when the program's data does. The pointers inside are the child's
/// final addresses -- the blob is built after the image is placed, so nothing about
/// it needs relocating.
///
/// It lives and dies with the slot: no allocation, and slot_free takes it with the
/// image.
fn write_argv(loaded: &crate::fs::elf::LoadedElf, name: &str, args: &[&str]) -> KResult<usize> {
    let argc = 1 + args.len();
    let ptrs_len = 4 + (argc + 1) * 4;
    let strs_len = name.len() + 1 + args.iter().map(|a| a.len() + 1).sum::<usize>();
    let blob_len = (ptrs_len + strs_len + 3) & !3;

    let slot = crate::mm::psram_exec::SLOT_SIZE as usize;
    if blob_len > slot {
        return Err(KError::NoSpace);
    }
    let off = slot - blob_len;
    // The image's .bss ends at data_size; below that is the program's own memory.
    if off < loaded.data_size as usize {
        return Err(KError::NoSpace);
    }

    let base = loaded.data_base as usize + off;
    let mut ptr_at = base + 4;
    let mut str_at = base + ptrs_len;

    unsafe {
        *(base as *mut u32) = argc as u32;
        for s in core::iter::once(name).chain(args.iter().copied()) {
            *(ptr_at as *mut u32) = str_at as u32;
            core::ptr::copy_nonoverlapping(s.as_ptr(), str_at as *mut u8, s.len());
            *((str_at + s.len()) as *mut u8) = 0;
            ptr_at += 4;
            str_at += s.len() + 1;
        }
        // argv is NULL-terminated as well as counted, so a program can walk it
        // either way.
        *(ptr_at as *mut u32) = 0;
    }

    // No sync_caches: this is the data bus on both sides. The image needed it
    // because its code was written through the data alias and fetched through the
    // instruction bus; argv is written and read through the same cache.
    Ok(base)
}

/// Loads a program and arranges its stdio, without running it.
///
/// `stdin`/`stdout` are fds in THIS shell's table; the child inherits a copy of the
/// table, so the same numbers name the same files there, and they get duplicated
/// onto 0 and 1 before it is unblocked. `close_in_child` names fds the child must
/// not keep -- pipe ends belonging to other stages. Leaving one open means the pipe
/// still has a writer and its reader never sees EOF.
///
/// Returns the child's pid, already runnable. Errors return a shell exit code.
fn spawn_child(
    name: &str,
    args: &[&str],
    stdin: vfs::Fd,
    stdout: vfs::Fd,
    close_in_child: &[vfs::Fd],
) -> Result<scheduler::process::Pid, i32> {
    let path = if name.contains('/') {
        resolve(name)
    } else {
        format!("/bin/{}", name)
    };

    let loaded = match crate::fs::elf::load_elf(&path) {
        Ok(l) => l,
        Err(KError::NotFound) => {
            eprint_line(&format!("shell: command not found: {}", name));
            return Err(127);
        }
        Err(e) => {
            eprint_line(&format!("shell: {}: {}", name, err_str(e)));
            return Err(126);
        }
    };

    let argv_ptr = match write_argv(&loaded, name, args) {
        Ok(p) => p,
        Err(e) => {
            crate::mm::psram_exec::slot_free(loaded.slot);
            eprint_line(&format!("shell: {}: {}", name, err_str(e)));
            return Err(126);
        }
    };

    let entry: fn(usize) = unsafe { core::mem::transmute(loaded.entry as usize) };
    let tid = match scheduler::spawn_blocked(
        name,
        entry,
        argv_ptr,
        layout::DEFAULT_STACK_SIZE,
        EXEC_PRIO,
        true,
    ) {
        Ok(t) => t,
        Err(e) => {
            // Nothing owns the image yet.
            crate::mm::psram_exec::slot_free(loaded.slot);
            eprint_line(&format!("shell: {}: cannot spawn ({})", name, err_str(e)));
            return Err(126);
        }
    };

    // register_process finds the parent by scanning for the CALLING task's tid, so
    // the parent is this shell. The child therefore inherits the shell's fd table --
    // and with it the session's channel on 0/1/2 -- and its cwd, without a line here
    // to arrange it. That inheritance is the whole point of giving the shell a pid.
    //
    // The slot rides on the process: reaping it is what returns the slot.
    let pid = scheduler::process::register_process(name, tid, true, Some(loaded.slot));

    // Between register_process (which cloned the table) and unblock_task (after
    // which the child could run) is the only window where its table can be edited.
    if stdin != STDIN {
        let _ = vfs::dup2_in(pid, stdin, STDIN);
    }
    if stdout != STDOUT {
        let _ = vfs::dup2_in(pid, stdout, STDOUT);
    }
    for &fd in close_in_child {
        vfs::close_in(pid, fd);
    }

    scheduler::unblock_task(tid);
    Ok(pid)
}

/// Waits for every pid and returns the LAST one's status, the way a shell reports a
/// pipeline.
///
/// Order does not matter, and not because of the scheduler: sys_wait reaps whichever
/// child has become a zombie, not a named one. So this just reaps as many times as
/// there are children and picks out the one it was asked about. That is also what
/// makes a pipeline drain -- an upstream stage's fds are only released when its
/// process is reaped, and until they are, the pipe still has a writer and the
/// downstream stage waits.
fn wait_all(pids: &[scheduler::process::Pid]) -> i32 {
    let last = match pids.last() {
        Some(p) => *p,
        None => return 0,
    };
    let mut last_status = 0;
    for _ in 0..pids.len() {
        let mut status: i32 = 0;
        let reaped = crate::syscall::invoke(
            crate::syscall::Syscall::Wait as usize,
            [&mut status as *mut i32 as usize, 0, 0, 0, 0, 0],
        );
        if reaped < 0 {
            eprint_line(&format!("shell: wait failed ({})", reaped));
            break;
        }
        if reaped as u32 == last {
            last_status = status;
        }
    }
    last_status
}

/// Blocks until the child exits, and returns its status.
///
/// Goes through the trap rather than calling sys_wait directly, because the trap IS
/// the blocking: sys_wait parks the task and sets the restart flag, and the trap
/// epilogue then leaves PC on the `syscall` instruction so it re-executes on wake.
/// Called as a plain function it would return 0 immediately, having waited for
/// nothing.
///
/// `status` is a kernel stack address. sys_wait writes it after a null check and
/// nothing else -- no syscall in this kernel validates a user pointer -- so it
/// simply works. That is also why pointer validation has to land before userland
/// can issue syscalls of its own.
fn wait_for(pid: scheduler::process::Pid, name: &str) -> i32 {
    let mut status: i32 = 0;
    let reaped = crate::syscall::invoke(
        crate::syscall::Syscall::Wait as usize,
        [&mut status as *mut i32 as usize, 0, 0, 0, 0, 0],
    );
    if reaped < 0 {
        eprint_line(&format!("shell: {}: wait failed ({})", name, reaped));
        return 1;
    }
    // sys_wait reaps whichever child is a zombie, not a named one. try_exec blocks,
    // so the shell only ever has one -- if that stops being true this silently
    // returns the wrong program's status.
    if reaped as u32 != pid {
        eprint_line(&format!("shell: {}: reaped pid {} but expected {}", name, reaped, pid));
    }
    status
}

/// Runs `stages` connected by pipes and returns the last one's status.
///
/// Every stage is a program from /bin, never a built-in, even when a built-in of
/// the same name exists. A built-in runs inside the shell's own task, so it cannot
/// run concurrently with the rest -- and running the stages one after another is not
/// a simplification but a deadlock: the first would fill the 4 KB pipe and block
/// forever with nobody draining it. There is no fork here to escape with.
///
/// The visible cost is that `ls /tmp` (built-in, honours the path) and
/// `ls /tmp | cat` (/bin/ls, ignores it) do different things until the userland
/// programs learn argv the way echo just did.
pub fn run_pipeline(stages: &[super::parser::Command]) -> i32 {
    // Check every stage before touching anything. A missing third stage should not
    // leave two slots taken and a pipe half built.
    for s in stages {
        let path = if s.name.contains('/') {
            resolve(&s.name)
        } else {
            format!("/bin/{}", s.name)
        };
        if vfs::mount::resolve(&path).is_err() {
            eprint_line(&format!(
                "shell: {}: not found in /bin (a pipeline stage cannot be a built-in)",
                s.name
            ));
            return 127;
        }
    }

    // One pipe between each pair. The fds live in this shell's table; every child
    // inherits a copy, which is why each has to be told to close the ones it does
    // not use.
    let mut pipes: Vec<(vfs::Fd, vfs::Fd)> = Vec::new();
    for _ in 1..stages.len() {
        match vfs::create_pipe() {
            Ok(p) => pipes.push(p),
            Err(e) => {
                eprint_line(&format!("shell: pipe failed ({})", err_str(e)));
                close_all(&pipes);
                return 1;
            }
        }
    }

    let mut all_fds: Vec<vfs::Fd> = Vec::new();
    for &(r, w) in &pipes {
        all_fds.push(r);
        all_fds.push(w);
    }

    let last = stages.len() - 1;
    let mut pids = Vec::new();
    let mut redirects: Vec<vfs::Fd> = Vec::new();

    for (i, s) in stages.iter().enumerate() {
        let stdin = if i == 0 { STDIN } else { pipes[i - 1].0 };

        // A stage's own `>` beats the pipe, which is what a real shell does:
        // `ls > f | cat` sends ls to the file and leaves cat with an empty pipe.
        let stdout = match open_redirect(&s.redirect) {
            Ok(Some(fd)) => {
                redirects.push(fd);
                fd
            }
            Ok(None) => {
                if i == last {
                    STDOUT
                } else {
                    pipes[i].1
                }
            }
            Err(e) => {
                eprint_line(&format!("shell: {}: {}", s.name, err_str(e)));
                close_all(&pipes);
                for fd in &redirects {
                    let _ = vfs::close(*fd);
                }
                let _ = wait_all(&pids);
                return 1;
            }
        };

        let args: Vec<&str> = s.args.iter().map(|a| a.as_str()).collect();
        match spawn_child(&s.name, &args, stdin, stdout, &all_fds) {
            Ok(pid) => pids.push(pid),
            Err(code) => {
                // Whatever already started still has to be drained, or its slots and
                // processes leak.
                close_all(&pipes);
                for fd in &redirects {
                    let _ = vfs::close(*fd);
                }
                let _ = wait_all(&pids);
                return code;
            }
        }
    }

    // The shell must drop its own ends now. While it still holds a write end the
    // pipe has a writer, and the stage reading it would wait for an EOF that never
    // comes.
    close_all(&pipes);
    for fd in &redirects {
        let _ = vfs::close(*fd);
    }

    wait_all(&pids)
}

fn close_all(pipes: &[(vfs::Fd, vfs::Fd)]) {
    for &(r, w) in pipes {
        let _ = vfs::close(r);
        let _ = vfs::close(w);
    }
}

/// Opens a stage's `>` target, or None when it has none.
fn open_redirect(redirect: &Redirect) -> KResult<Option<vfs::Fd>> {
    let (path, extra) = match redirect {
        Redirect::None => return Ok(None),
        Redirect::Truncate(p) => (p, OpenFlags::TRUNC.0),
        Redirect::Append(p) => (p, OpenFlags::APPEND.0),
    };
    let flags = OpenFlags(OpenFlags::WRONLY.0 | OpenFlags::CREATE.0 | extra);
    Ok(Some(vfs::open(&resolve(path), flags)?))
}

pub(crate) fn eprint_syntax_error(s: &str) {
    eprint_line(s);
}

pub fn dispatch(name: &str, args: &[&str]) -> i32 {
    match name {
        "echo" => cmd_echo(args),
        "help" => cmd_help(args),
        "clear" => cmd_clear(),
        "uptime" => cmd_uptime(),
        "free" => cmd_free(),
        "ps" => cmd_ps(),
        "reboot" => cmd_reboot(),
        "ls" => cmd_ls(args),
        "cd" => cmd_cd(args),
        "pwd" => cmd_pwd(),
        "cat" => cmd_cat(args),
        "mkdir" => cmd_mkdir(args),
        "touch" => cmd_touch(args),
        "rm" => cmd_rm(args),
        "write" => cmd_write(args),
        "i2c" => cmd_i2c(args),
        "spi" => cmd_spi(args),
        "ota" => cmd_ota(args),
        "syscalltest" => cmd_syscalltest(),
        "smp" => cmd_smp(),
        "pms" => cmd_pms(args),
        "power" => cmd_power(args),
        "sha256" => cmd_sha256(args),
        "ble" => cmd_ble(args),
        "wifi" => cmd_wifi(args),
        "ip" => cmd_ip(args),
        "nmcli" => cmd_nmcli(args),
        "sudo" => cmd_sudo(args),
        "" => 0,
        other => try_exec(other, args),
    }
}

fn cmd_echo(args: &[&str]) -> i32 {
    let mut newline = true;
    let mut start = 0usize;
    if let Some(&first) = args.first() {
        if first == "-n" {
            newline = false;
            start = 1;
        }
    }
    let mut out = String::new();

    for (i, a) in args[start..].iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(a);
    }
    if newline {
        emit_line(&out);
    } else {
        emit_str(&out);
    }
    0
}

fn cmd_wifi(args: &[&str]) -> i32 {
    use crate::drivers::wifi;
    match args {
        [] | ["status"] => {
            emit_line(&format!("state:  {:?}", wifi::status()));
            emit_line(&format!(
                "ssid:   {}",
                wifi::current_ssid().unwrap_or_else(|| String::from("(none)"))
            ));
            match wifi::current_ip() {
                Some(ip) => emit_line(&format!("ip:     {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])),
                None => emit_line("ip:     (none)"),
            }
            0
        }
        ["scan"] | ["list"] => {
            emit_line("Scanning for networks (briefly drops the Wi-Fi link)...");
            wifi::request_scan();
            let start = timer::uptime_ms();
            while wifi::scan_state() == wifi::SCAN_RUNNING {
                if timer::uptime_ms().saturating_sub(start) > 20_000 {
                    eprint_line(&format!(
                        "wifi: scan timed out [diag: {}]",
                        wifi::scan_diag()
                    ));
                    return 1;
                }
                scheduler::yield_now();
            }
            if wifi::scan_state() == wifi::SCAN_ERROR {
                eprint_line(&format!("wifi: scan failed [diag: {}]", wifi::scan_diag()));
                return 1;
            }
            let mut aps = wifi::scan_results();
            aps.sort_by(|a, b| b.rssi.cmp(&a.rssi));
            emit_line(&format!(
                "{:<32} {:>5}  {:>3}  {}",
                "SSID", "RSSI", "CH", "SEC"
            ));
            for ap in aps.iter() {
                let name = if ap.ssid.is_empty() {
                    "(hidden)"
                } else {
                    ap.ssid.as_str()
                };
                emit_line(&format!(
                    "{:<32} {:>5}  {:>3}  {}",
                    name,
                    ap.rssi,
                    ap.channel,
                    if ap.secured { "WPA" } else { "open" }
                ));
            }
            if aps.is_empty() {
                emit_line("(no networks found)");
            }
            0
        }
        ["connect", ssid] => {
            emit_line(&format!("Connecting to '{}' (open network)...", ssid));
            wifi::request_connect(String::from(*ssid), String::new());
            emit_line("(use 'wifi status' to check the result)");
            0
        }
        ["connect", ssid, password] => {
            emit_line(&format!("Connecting to '{}'...", ssid));
            wifi::request_connect(String::from(*ssid), String::from(*password));
            emit_line("(use 'wifi status' to check the result)");
            0
        }
        ["disconnect"] => {
            wifi::request_disconnect();
            emit_line("Disconnecting...");
            0
        }
        ["forget"] => match crate::drivers::wifi_store::clear() {
            Ok(()) => {
                emit_line("Saved Wi-Fi credentials cleared (reboot uses compiled defaults).");
                0
            }
            Err(e) => {
                eprint_line(&format!("wifi: could not clear credentials: {:?}", e));
                1
            }
        },
        _ => {
            emit_line(
                "usage: wifi status | scan | connect \"SSID\" [PASSWORD] | disconnect | forget",
            );
            1
        }
    }
}

fn cmd_ip(_args: &[&str]) -> i32 {
    use crate::drivers::wifi;

    match wifi::current_ip() {
        Some(ip) => emit_line(&format!(
            "wlan0: {}.{}.{}.{}  ssid \"{}\"  state {:?}",
            ip[0],
            ip[1],
            ip[2],
            ip[3],
            wifi::current_ssid().unwrap_or_else(|| String::from("(none)")),
            wifi::status()
        )),
        None => emit_line(&format!("wlan0: <no address>  state {:?}", wifi::status())),
    }
    0
}

fn cmd_sudo(args: &[&str]) -> i32 {
    if args.is_empty() {
        eprint_line("sudo: usage: sudo COMMAND [ARGS...]");
        return 1;
    }
    dispatch(args[0], &args[1..])
}

fn cmd_nmcli(args: &[&str]) -> i32 {
    match args {
        ["device", "status"] | ["dev", "status"] | ["general", "status"] | ["g", "status"] => {
            cmd_wifi(&["status"])
        }
        ["radio", ..] => {
            emit_line("wifi radio is always on");
            0
        }
        ["device", "wifi", "list"]
        | ["dev", "wifi", "list"]
        | ["device", "wifi"]
        | ["dev", "wifi"] => cmd_wifi(&["scan"]),
        ["device", "wifi", "connect", rest @ ..] | ["dev", "wifi", "connect", rest @ ..] => {
            let ssid = match rest.first() {
                Some(s) => *s,
                None => {
                    eprint_line("nmcli: missing SSID");
                    return 1;
                }
            };
            let pass = match rest {
                [_, "password", p, ..] => *p,
                _ => "",
            };
            if pass.is_empty() {
                cmd_wifi(&["connect", ssid])
            } else {
                cmd_wifi(&["connect", ssid, pass])
            }
        }
        _ => {
            emit_line("nmcli (EspressoOS shim). Supported:");
            emit_line("  nmcli device status");
            emit_line("  nmcli device wifi list");
            emit_line("  nmcli device wifi connect \"SSID\" password \"PASS\"");
            emit_line("(native equivalent: the 'wifi' command)");
            0
        }
    }
}

fn cmd_help(_args: &[&str]) -> i32 {
    emit_line("Available commands:");
    emit_line("  echo [-n] TEXT...     print text");
    emit_line("  help                  show this help");
    emit_line("  clear                 clear the screen");
    emit_line("  uptime                time since boot");
    emit_line("  free                  kernel heap usage");
    emit_line("  ps                    tasks (current task)");
    emit_line("  reboot                reboot the system");
    emit_line("  ls [PATH]             list a directory");
    emit_line("  cd [PATH]             change directory (default /)");
    emit_line("  pwd                   show the current directory");
    emit_line("  cat FILE...           show the contents of files");
    emit_line("  mkdir DIR...          create directories");
    emit_line("  touch FILE...         create empty files");
    emit_line("  rm FILE...            remove files");
    emit_line("  write FILE TEXT       write TEXT to FILE (truncate)");
    emit_line("  i2c scan|read|write   I2C bus (/dev/i2c0)");
    emit_line("  spi transfer B0...    SPI bus (/dev/spi0)");
    emit_line("  ota status|set|rx|apply  A/B update (otadata + OTA:3300)");
    emit_line("  syscalltest           exercise the syscall ABI");
    emit_line("  smp                   multicore status (SMP)");
    emit_line("  pms [world1]          memory protection (PMS)");
    emit_line("  power sleep|deep-sleep [seconds]  power management");
    emit_line("  sha256 [text]         hardware SHA-256 hashing");
    emit_line("  ble status|advertise  Bluetooth LE management and advertising");
    emit_line("  wifi status|scan|connect \"SSID\" [PASS]|disconnect   Wi-Fi management");
    emit_line("  ip                    show the wlan0 address");
    emit_line("");
    emit_line("Redirection: '> file' (truncate) and '>> file' (append).");
    0
}

fn cmd_clear() -> i32 {
    emit_str("\x1b[2J\x1b[H");
    0
}

fn cmd_uptime() -> i32 {
    let ms = timer::uptime_ms();
    let total_s = ms / 1000;
    let days = total_s / 86_400;
    let hours = (total_s % 86_400) / 3_600;
    let mins = (total_s % 3_600) / 60;
    let secs = total_s % 60;
    emit_line(&format!(
        "up {}d {:02}:{:02}:{:02} ({} ms)",
        days, hours, mins, secs, ms
    ));
    0
}

fn cmd_free() -> i32 {
    let s = mm::stats();
    emit_line("            total         used         free");
    emit_line(&format!(
        "heap  {:>11}  {:>11}  {:>11}",
        s.total, s.used, s.free
    ));
    // Userland images do not live on the heap -- they get a slot out of the
    // reserved PSRAM region -- so the line above would never move no matter how
    // many programs ran, and a slot leak would be invisible until the 33rd exec
    // failed with "busy".
    let used = crate::mm::psram_exec::slots_in_use();
    emit_line(&format!(
        "slots {:>11}  {:>11}  {:>11}",
        crate::mm::psram_exec::SLOT_COUNT,
        used,
        crate::mm::psram_exec::SLOT_COUNT - used
    ));
    0
}

fn cmd_ps() -> i32 {
    let tid = scheduler::current();
    emit_line("  TID  STATE     NAME");
    emit_line(&format!("{:>5}  Running   (current)", tid));
    0
}

fn cmd_reboot() -> i32 {
    eprint_line("Rebooting the system...");

    esp_hal::reset::software_reset();

    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

fn cmd_ls(args: &[&str]) -> i32 {
    let raw = args.first().copied().unwrap_or(".");
    let path = resolve(raw);
    match vfs::readdir(&path) {
        Ok(entries) => {
            for e in &entries {
                emit_line(&format_entry(e));
            }
            0
        }
        Err(e) => {
            eprint_line(&format!("ls: {}: {}", raw, err_str(e)));
            1
        }
    }
}

fn cmd_cd(args: &[&str]) -> i32 {
    let target = args.first().copied().unwrap_or("/");
    let abs = resolve(target);

    match vfs::readdir(&abs) {
        Ok(_) => {
            scheduler::process::cwd_set(&abs);
            0
        }
        Err(e) => {
            eprint_line(&format!("cd: {}: {}", target, err_str(e)));
            1
        }
    }
}

fn cmd_pwd() -> i32 {
    emit_line(&cwd_get());
    0
}

fn format_entry(e: &DirEntry) -> String {
    let tag = match e.kind {
        InodeKind::Dir => "/",
        InodeKind::Device => "@",
        InodeKind::Symlink => "~",
        InodeKind::File => "",
    };
    format!("{}{}", e.name, tag)
}

fn cmd_cat(args: &[&str]) -> i32 {
    if args.is_empty() {
        eprint_line("usage: cat FILE...");
        return 2;
    }
    let mut status = 0;
    for path in args {
        if let Err(e) = cat_one(&resolve(path)) {
            eprint_line(&format!("cat: {}: {}", path, err_str(e)));
            status = 1;
        }
    }
    status
}

fn cat_one(path: &str) -> KResult<()> {
    let fd = vfs::open(path, OpenFlags::RDONLY)?;
    let mut buf = [0u8; 128];
    loop {
        match vfs::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = buf.get(..n).unwrap_or(&[]);
                emit(chunk);
            }
            Err(e) => {
                let _ = vfs::close(fd);
                return Err(e);
            }
        }
    }
    let _ = vfs::close(fd);
    Ok(())
}

fn cmd_mkdir(args: &[&str]) -> i32 {
    if args.is_empty() {
        eprint_line("usage: mkdir DIRECTORY...");
        return 2;
    }
    let mut status = 0;
    for path in args {
        if let Err(e) = vfs::mkdir(&resolve(path)) {
            eprint_line(&format!("mkdir: {}: {}", path, err_str(e)));
            status = 1;
        }
    }
    status
}

fn cmd_touch(args: &[&str]) -> i32 {
    if args.is_empty() {
        eprint_line("usage: touch FILE...");
        return 2;
    }
    let mut status = 0;
    let flags = OpenFlags(OpenFlags::WRONLY.0 | OpenFlags::CREATE.0);
    for path in args {
        match vfs::open(&resolve(path), flags) {
            Ok(fd) => {
                let _ = vfs::close(fd);
            }
            Err(e) => {
                eprint_line(&format!("touch: {}: {}", path, err_str(e)));
                status = 1;
            }
        }
    }
    status
}

fn cmd_rm(args: &[&str]) -> i32 {
    if args.is_empty() {
        eprint_line("usage: rm FILE...");
        return 2;
    }
    let mut status = 0;
    for path in args {
        if let Err(e) = vfs::unlink(&resolve(path)) {
            eprint_line(&format!("rm: {}: {}", path, err_str(e)));
            status = 1;
        }
    }
    status
}

fn cmd_write(args: &[&str]) -> i32 {
    let path = match args.first() {
        Some(p) => *p,
        None => {
            eprint_line("usage: write FILE TEXT...");
            return 2;
        }
    };
    let mut text = String::new();
    for (i, a) in args.iter().skip(1).enumerate() {
        if i > 0 {
            text.push(' ');
        }
        text.push_str(a);
    }
    text.push('\n');

    let flags = OpenFlags(OpenFlags::WRONLY.0 | OpenFlags::CREATE.0 | OpenFlags::TRUNC.0);
    let fd = match vfs::open(&resolve(path), flags) {
        Ok(fd) => fd,
        Err(e) => {
            eprint_line(&format!("write: {}: {}", path, err_str(e)));
            return 1;
        }
    };
    let res = vfs::write(fd, text.as_bytes());
    let _ = vfs::close(fd);
    match res {
        Ok(_) => 0,
        Err(e) => {
            eprint_line(&format!("write: {}: {}", path, err_str(e)));
            1
        }
    }
}

fn parse_u8_hex(s: &str) -> Option<u8> {
    let t = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u8::from_str_radix(t, 16).ok()
}

fn zeroed(len: usize) -> Vec<u8> {
    let mut v = Vec::new();
    v.resize(len, 0u8);
    v
}

fn hex_bytes(data: &[u8]) -> String {
    let mut s = String::new();
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn cmd_i2c(args: &[&str]) -> i32 {
    use crate::drivers::i2c;
    match args.first().copied() {
        Some("scan") => {
            if !i2c::is_ready() {
                eprint_line("i2c: bus not initialized");
                return 1;
            }
            emit_line("Scanning I2C bus (0x08..0x77)...");
            let mut found = 0u32;
            for addr in i2c::SCAN_FIRST..=i2c::SCAN_LAST {
                if i2c::probe(addr) {
                    emit_line(&format!("  device at 0x{:02x}", addr));
                    found += 1;
                }
            }
            emit_line(&format!("{} device(s) found", found));
            0
        }
        Some("read") => {
            let addr = args.get(1).and_then(|s| parse_u8_hex(s));
            let len = args.get(2).and_then(|s| s.parse::<usize>().ok());
            match (addr, len) {
                (Some(addr), Some(len)) if (1..=64).contains(&len) => {
                    let mut buf = zeroed(len);
                    match i2c::read(addr, &mut buf) {
                        Ok(()) => {
                            emit_line(&hex_bytes(&buf));
                            0
                        }
                        Err(e) => {
                            eprint_line(&format!("i2c read: {}", err_str(e)));
                            1
                        }
                    }
                }
                _ => {
                    eprint_line("usage: i2c read ADDR_HEX LEN(1..64)");
                    2
                }
            }
        }
        Some("write") => {
            let addr = args.get(1).and_then(|s| parse_u8_hex(s));
            match addr {
                Some(addr) if args.len() >= 3 => {
                    let mut data = Vec::new();
                    for s in &args[2..] {
                        match parse_u8_hex(s) {
                            Some(b) => data.push(b),
                            None => {
                                eprint_line(&format!("i2c write: invalid byte: {}", s));
                                return 2;
                            }
                        }
                    }
                    match i2c::write(addr, &data) {
                        Ok(()) => 0,
                        Err(e) => {
                            eprint_line(&format!("i2c write: {}", err_str(e)));
                            1
                        }
                    }
                }
                _ => {
                    eprint_line("usage: i2c write ADDR_HEX B0 [B1 ...]");
                    2
                }
            }
        }
        _ => {
            eprint_line("usage: i2c scan | i2c read ADDR LEN | i2c write ADDR B0 ...");
            2
        }
    }
}

fn cmd_spi(args: &[&str]) -> i32 {
    use crate::drivers::spi;
    match args.first().copied() {
        Some("transfer") => {
            if args.len() < 2 {
                eprint_line("usage: spi transfer B0 [B1 ...]");
                return 2;
            }
            let mut tx = Vec::new();
            for s in &args[1..] {
                match parse_u8_hex(s) {
                    Some(b) => tx.push(b),
                    None => {
                        eprint_line(&format!("spi transfer: invalid byte: {}", s));
                        return 2;
                    }
                }
            }
            let mut rx = zeroed(tx.len());
            match spi::transfer(&tx, &mut rx) {
                Ok(()) => {
                    emit_line(&hex_bytes(&rx));
                    0
                }
                Err(e) => {
                    eprint_line(&format!("spi transfer: {}", err_str(e)));
                    1
                }
            }
        }
        _ => {
            eprint_line("usage: spi transfer B0 [B1 ...]");
            2
        }
    }
}

fn slot_name(s: crate::ota::Slot) -> &'static str {
    match s {
        crate::ota::Slot::Factory => "factory",
        crate::ota::Slot::Ota0 => "ota0",
    }
}

fn parse_slot(s: &str) -> Option<crate::ota::Slot> {
    match s {
        "factory" => Some(crate::ota::Slot::Factory),
        "ota0" | "ota_0" => Some(crate::ota::Slot::Ota0),
        _ => None,
    }
}

fn ota_state_str(raw: u32) -> &'static str {
    use crate::ota::OtaImgState::*;
    match crate::ota::OtaImgState::from_raw(raw) {
        New => "new",
        PendingVerify => "pending-verify",
        Valid => "valid",
        Invalid => "invalid",
        Aborted => "aborted",
        Undefined => "undef",
    }
}

fn cmd_ota(args: &[&str]) -> i32 {
    use crate::ota;
    match args.first().copied() {
        Some("status") => {
            emit_line(&format!("active slot: {}", slot_name(ota::active_slot())));
            match ota::otadata_entries() {
                Ok(entries) => {
                    for (i, e) in entries.iter().enumerate() {
                        let seq = if e.ota_seq == 0xFFFF_FFFF {
                            String::from("(empty)")
                        } else {
                            format!("{}", e.ota_seq)
                        };
                        emit_line(&format!(
                            "  otadata[{}]: seq={} state={} ({})",
                            i,
                            seq,
                            ota_state_str(e.ota_state),
                            if e.is_valid() {
                                "valid"
                            } else {
                                "invalid/empty"
                            }
                        ));
                    }
                    0
                }
                Err(e) => {
                    eprint_line(&format!("ota status: {}", err_str(e)));
                    1
                }
            }
        }
        Some("set") => match args.get(1).copied().and_then(parse_slot) {
            Some(slot) => match ota::set_boot_slot(slot) {
                Ok(()) => {
                    emit_line(&format!("marked for boot: {}", slot_name(slot)));
                    emit_line("note: the actual switch at boot requires a bootloader that");
                    emit_line("honors otadata; see the OTA section of the README.");
                    0
                }
                Err(e) => {
                    eprint_line(&format!("ota set: {}", err_str(e)));
                    1
                }
            },
            None => {
                eprint_line("usage: ota set factory|ota0");
                2
            }
        },
        Some("rx") => {
            let n = ota::rx_len();
            if n == 0 {
                emit_line("no image in buffer (send it: nc <ip> 3300 < firmware.bin)");
            } else {
                emit_line(&format!("image in buffer: {} bytes (use 'ota apply')", n));
            }
            0
        }
        Some("apply") => {
            emit_line("Flashing received image to the inactive slot...");
            emit_line("(writes several MB; with WiFi active it may cut the radio)");
            match ota::apply_buffered() {
                Ok(slot) => {
                    emit_line(&format!(
                        "OK: image written to {} and marked for boot",
                        slot_name(slot)
                    ));
                    emit_line("(effective only with a bootloader that honors otadata)");
                    0
                }
                Err(e) => {
                    eprint_line(&format!("ota apply: {}", err_str(e)));
                    1
                }
            }
        }
        _ => {
            eprint_line("usage: ota status | set factory|ota0 | rx | apply");
            2
        }
    }
}

fn cmd_syscalltest() -> i32 {
    use crate::syscall::{invoke, Syscall};

    let up = invoke(Syscall::UptimeMs.number(), [0; 6]);
    emit_line(&format!("SYS_UptimeMs -> {} ms", up));

    let free = invoke(Syscall::Sbrk.number(), [0; 6]);
    emit_line(&format!("SYS_Sbrk(free) -> {} bytes", free));

    let path = "/dev/console";
    let flags = OpenFlags::WRONLY.0 as usize;
    let fd = invoke(
        Syscall::Open.number(),
        [path.as_ptr() as usize, path.len(), flags, 0, 0, 0],
    );
    if fd >= 0 {
        let msg = b"  <- line written by SYS_Write to /dev/console\r\n";
        let n = invoke(
            Syscall::Write.number(),
            [fd as usize, msg.as_ptr() as usize, msg.len(), 0, 0, 0],
        );
        let _ = invoke(Syscall::Close.number(), [fd as usize, 0, 0, 0, 0, 0]);
        emit_line(&format!(
            "SYS_Open/Write/Close /dev/console -> fd={} written={}",
            fd, n
        ));
    } else {
        emit_line(&format!("SYS_Open /dev/console -> error {}", fd));
    }

    let y = invoke(Syscall::Yield.number(), [0; 6]);
    emit_line(&format!("SYS_Yield -> {}", y));

    if cfg!(feature = "syscall-trap") {
        emit_line("(via real `syscall` instruction / CPU trap)");
    } else {
        emit_line("(via software gate; --features syscall-trap for the real trap)");
    }
    0
}

fn cmd_smp() -> i32 {
    use crate::scheduler::core_sync;
    emit_line(&format!(
        "core serving the shell: core {}",
        core_sync::current_core_id()
    ));
    if cfg!(feature = "smp") {
        if core_sync::is_running() {
            emit_line(&format!(
                "SMP: APP_CPU (core 1) active — ticks core1 = {}",
                core_sync::core1_ticks()
            ));
        } else {
            emit_line("SMP: compiled but the APP_CPU didn't start");
        }
    } else {
        emit_line("SMP: not compiled (use: cargo build --release --features smp)");
    }
    0
}

fn cmd_pms(args: &[&str]) -> i32 {
    if !cfg!(feature = "pms") {
        eprint_line("pms: not compiled (use: cargo build --release --features pms)");
        return 1;
    }
    match args.first().copied() {
        Some("world1") => {
            emit_line("PMS: applying World-1 restriction (experimental)...");
            match crate::mm::mpu::protect_world1_wx() {
                Some(s) => {
                    emit_line(&s);
                    0
                }
                None => {
                    eprint_line("pms: not available");
                    1
                }
            }
        }
        _ => match crate::mm::mpu::report() {
            Some(s) => {
                emit_line(&s);
                0
            }
            None => {
                eprint_line("pms: not available");
                1
            }
        },
    }
}

fn cmd_power(args: &[&str]) -> i32 {
    let mode = args.first().copied();
    let secs = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(5);

    match mode {
        Some("sleep") => {
            crate::drivers::power::enter_light_sleep(secs);
            0
        }
        Some("deep-sleep") => {
            crate::drivers::power::enter_deep_sleep(secs);
        }
        _ => {
            eprint_line("usage: power sleep [seconds] | power deep-sleep [seconds]");
            1
        }
    }
}

fn cmd_sha256(args: &[&str]) -> i32 {
    let input = args.first().copied().unwrap_or("hello");
    let hash = crate::drivers::crypto::sha256(input.as_bytes());
    let mut hex = String::new();
    for &b in &hash {
        hex.push_str(&format!("{:02x}", b));
    }
    emit_line(&format!("SHA256(\"{}\") = {}", input, hex));
    0
}

fn cmd_ble(args: &[&str]) -> i32 {
    let sub = args.first().copied();
    match sub {
        Some("status") => {
            if crate::drivers::ble::is_advertising() {
                emit_line("BLE: Actively advertising as 'EspressoOS'");
            } else {
                emit_line("BLE: Inactive (not advertising)");
            }
            0
        }
        Some("advertise") => {
            crate::drivers::ble::start_advertising();
            0
        }
        _ => {
            eprint_line("usage: ble status | ble advertise");
            1
        }
    }
}

fn err_str(e: KError) -> &'static str {
    match e {
        KError::NoMem => "out of memory",
        KError::NotFound => "not found",
        KError::AlreadyExists => "already exists",
        KError::NotADirectory => "not a directory",
        KError::IsADirectory => "is a directory",
        KError::InvalidArgument => "invalid argument",
        KError::PermissionDenied => "permission denied",
        KError::NotSupported => "not supported",
        KError::WouldBlock => "would block",
        KError::Busy => "busy",
        KError::IoError => "I/O error",
        KError::BadFd => "invalid descriptor",
        KError::NameTooLong => "name too long",
        KError::NoSpace => "out of space",
        KError::Corrupt => "corrupt data",
        KError::Timeout => "timed out",
        KError::Fault => "invalid address",
        KError::TableFull => "table full",
    }
}
