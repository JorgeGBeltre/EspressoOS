#![allow(dead_code)]

pub mod commands;
pub mod parser;

use crate::prelude::*;
use crate::scheduler;
use crate::vfs;
use alloc::format;

use commands::{write_all, STDIN, STDOUT};

const MAX_LINE: usize = 256;

pub(crate) const MAX_LINE_LEN: usize = MAX_LINE;

/// Runs one interactive session on the caller's own fds, and returns when stdin
/// reports end of session.
///
/// There is exactly one of these for both the serial console and SSH. Nothing
/// here knows which it is: the session's channel is fd 0/1/2 of the calling
/// process, seeded before this task was ever unblocked. `\n` goes out bare --
/// the channel adds the `\r`.
pub fn run_session(user: Option<&str>) {
    out(b"\nEspressoOS shell. Type 'help' to see the commands.\n");

    let mut line = String::new();
    loop {
        let cwd = commands::cwd_get();
        let display_cwd = if cwd == "/" { "~" } else { cwd.as_str() };
        let prompt = match user {
            Some(u) => format!("{}@EspressoOS:{}$ ", u, display_cwd),
            None => format!("EspressoOS:{}$ ", display_cwd),
        };
        out(prompt.as_bytes());

        line.clear();
        if !read_line(&mut line) {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "exit" || trimmed == "quit" || trimmed == "logout" {
            out(b"logout\n");
            break;
        }
        execute(trimmed);
    }
}

fn out(bytes: &[u8]) {
    write_all(STDOUT, bytes);
}

/// Returns false when the session is over (stdin hit EOF), true on a full line.
///
/// The serial channel never reports EOF, so the console shell loops forever; an
/// SSH channel does, the moment it is closed, which is what lets its task fall
/// off the end and exit.
fn read_line(buf: &mut String) -> bool {
    loop {
        let mut b = [0u8; 1];
        match vfs::read(STDIN, &mut b) {
            Ok(0) => return false,
            Ok(_) => match b[0] {
                b'\r' | b'\n' => {
                    out(b"\n");
                    return true;
                }
                0x08 | 0x7f => {
                    if buf.pop().is_some() {
                        out(b"\x08 \x08");
                    }
                }
                0x03 => {
                    buf.clear();
                    out(b"^C\n");
                    return true;
                }
                c if (0x20..0x7f).contains(&c) => {
                    if buf.len() < MAX_LINE {
                        buf.push(c as char);
                        out(&[c]);
                    }
                }
                _ => {}
            },
            // Nothing typed yet. Yielding is safe here: vfs::read holds no lock
            // once it returns.
            Err(KError::WouldBlock) => scheduler::yield_now(),
            Err(_) => return false,
        }
    }
}

pub(crate) fn execute_line(line: &str) {
    execute(line);
}

fn execute(line: &str) {
    match parser::parse_pipeline(line) {
        Ok(pipeline) => {
            if pipeline.is_empty() {
                return;
            }
            if pipeline.len() > 1 {
                // Every stage of a pipeline is a /bin program, so this path skips
                // the built-in table entirely. See commands::run_pipeline.
                commands::run_pipeline(&pipeline);
            } else if let Some(cmd) = pipeline.into_iter().next() {
                run_command(&cmd);
            }
        }
        Err(e) => {
            commands::eprint_syntax_error(&format!("shell: syntax error ({:?})", e));
        }
    }
}

fn run_command(cmd: &parser::Command) {
    let saved = match commands::begin_redirect(&cmd.redirect) {
        Ok(s) => s,
        Err(e) => {
            commands::eprint_syntax_error(&format!(
                "shell: could not open redirection target ({:?})",
                e
            ));
            return;
        }
    };

    let args: Vec<&str> = cmd.args.iter().map(|s| s.as_str()).collect();
    let _code = commands::dispatch(cmd.name.as_str(), &args);

    commands::end_redirect(saved);
}
