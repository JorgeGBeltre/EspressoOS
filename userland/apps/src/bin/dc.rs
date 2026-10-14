#![no_std]
#![no_main]

use libc::{arg, print, println, read};

struct DcStack {
    data: [i32; 32],
    top: usize,
}

impl DcStack {
    const fn new() -> Self {
        Self {
            data: [0; 32],
            top: 0,
        }
    }

    fn push(&mut self, val: i32) -> bool {
        if self.top < self.data.len() {
            self.data[self.top] = val;
            self.top += 1;
            true
        } else {
            println!("dc: stack overflow");
            false
        }
    }

    fn pop(&mut self) -> Option<i32> {
        if self.top > 0 {
            self.top -= 1;
            Some(self.data[self.top])
        } else {
            println!("dc: stack empty");
            None
        }
    }

    fn peek(&self) -> Option<i32> {
        if self.top > 0 {
            Some(self.data[self.top - 1])
        } else {
            println!("dc: stack empty");
            None
        }
    }

    fn clear(&mut self) {
        self.top = 0;
    }
}

fn print_num(val: i32) {
    if val < 0 {
        print!("-");
        print_dec((val as i64).unsigned_abs() as usize);
    } else {
        print_dec(val as usize);
    }
}

fn print_dec(mut n: usize) {
    if n == 0 {
        print!("0");
        return;
    }
    let mut buf = [0u8; 16];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        let s = core::slice::from_ref(&buf[i]);
        if let Ok(c) = core::str::from_utf8(s) {
            print!("{}", c);
        }
    }
}

fn process_tokens(input: &str, stack: &mut DcStack) -> bool {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace
        while i < len && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\r' || bytes[i] == b'\n') {
            i += 1;
        }
        if i >= len {
            break;
        }

        let b = bytes[i];
        if (b >= b'0' && b <= b'9') || (b == b'_' && i + 1 < len && bytes[i + 1] >= b'0' && bytes[i + 1] <= b'9') {
            let negative = b == b'_';
            if negative {
                i += 1;
            }
            let mut val: i32 = 0;
            while i < len && bytes[i] >= b'0' && bytes[i] <= b'9' {
                val = val.wrapping_mul(10).wrapping_add((bytes[i] - b'0') as i32);
                i += 1;
            }
            if negative {
                val = -val;
            }
            stack.push(val);
        } else {
            match b {
                b'+' => {
                    if let (Some(a), Some(b)) = (stack.pop(), stack.pop()) {
                        stack.push(b.wrapping_add(a));
                    }
                }
                b'-' => {
                    if let (Some(a), Some(b)) = (stack.pop(), stack.pop()) {
                        stack.push(b.wrapping_sub(a));
                    }
                }
                b'*' => {
                    if let (Some(a), Some(b)) = (stack.pop(), stack.pop()) {
                        stack.push(b.wrapping_mul(a));
                    }
                }
                b'/' => {
                    if let (Some(a), Some(b)) = (stack.pop(), stack.pop()) {
                        if a == 0 {
                            println!("dc: divide by zero");
                        } else {
                            stack.push(b / a);
                        }
                    }
                }
                b'%' => {
                    if let (Some(a), Some(b)) = (stack.pop(), stack.pop()) {
                        if a == 0 {
                            println!("dc: remainder by zero");
                        } else {
                            stack.push(b % a);
                        }
                    }
                }
                b'p' => {
                    if let Some(top) = stack.peek() {
                        print_num(top);
                        println!("");
                    }
                }
                b'n' => {
                    if let Some(top) = stack.pop() {
                        print_num(top);
                    }
                }
                b'f' => {
                    for idx in (0..stack.top).rev() {
                        print_num(stack.data[idx]);
                        println!("");
                    }
                }
                b'c' => {
                    stack.clear();
                }
                b'd' => {
                    if let Some(top) = stack.peek() {
                        stack.push(top);
                    }
                }
                b'r' => {
                    if let (Some(a), Some(b)) = (stack.pop(), stack.pop()) {
                        stack.push(a);
                        stack.push(b);
                    }
                }
                b'q' => {
                    return false; // quit
                }
                _ => {}
            }
            i += 1;
        }
    }
    true
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut stack = DcStack::new();

    let mut i = 1;
    let mut has_expr = false;

    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--help" || a == "-h" {
            println!("Usage: dc [OPTION] [EXPRESSION...]");
            println!("Desk Calculator — classic Unix reverse-Polish arithmetic evaluator");
            println!("  -e, --expression=EXPR  evaluate EXPR string");
            return 0;
        } else if (a == "-e" || a == "--expression") && i + 1 < argc {
            i += 1;
            let expr = unsafe { arg(argv, i) };
            if !process_tokens(expr, &mut stack) {
                return 0;
            }
            has_expr = true;
        } else if !a.starts_with('-') {
            if !process_tokens(a, &mut stack) {
                return 0;
            }
            has_expr = true;
        }
        i += 1;
    }

    if !has_expr {
        let mut buf = [0u8; 256];
        loop {
            let n = read(0, &mut buf);
            if n <= 0 {
                break;
            }
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                if !process_tokens(s, &mut stack) {
                    break;
                }
            }
        }
    }

    0
}
