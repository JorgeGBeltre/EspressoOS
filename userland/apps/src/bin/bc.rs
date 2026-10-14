#![no_std]
#![no_main]

use libc::{arg, print, println, read};

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

fn print_num(val: i64) {
    if val < 0 {
        print!("-");
        print_dec((-val) as usize);
    } else {
        print_dec(val as usize);
    }
    println!("");
}

fn precedence(op: u8) -> u8 {
    match op {
        b'+' | b'-' => 1,
        b'*' | b'/' | b'%' => 2,
        b'^' => 3,
        _ => 0,
    }
}

fn ipow(mut base: i64, mut exp: i64) -> i64 {
    if exp < 0 {
        return 0;
    }
    let mut res = 1i64;
    while exp > 0 {
        if (exp & 1) != 0 {
            res = res.wrapping_mul(base);
        }
        base = base.wrapping_mul(base);
        exp >>= 1;
    }
    res
}

fn apply_op(val_stack: &mut [i64; 32], val_top: &mut usize, op: u8) -> bool {
    if *val_top < 2 {
        return false;
    }
    let b = val_stack[*val_top - 1];
    let a = val_stack[*val_top - 2];
    *val_top -= 2;

    let res = match op {
        b'+' => a.wrapping_add(b),
        b'-' => a.wrapping_sub(b),
        b'*' => a.wrapping_mul(b),
        b'/' => {
            if b == 0 {
                println!("bc: divide by zero");
                return false;
            }
            a / b
        }
        b'%' => {
            if b == 0 {
                println!("bc: divide by zero");
                return false;
            }
            a % b
        }
        b'^' => ipow(a, b),
        _ => return false,
    };

    val_stack[*val_top] = res;
    *val_top += 1;
    true
}

fn eval_expr(expr: &str) {
    let bytes = expr.as_bytes();
    let n = bytes.len();
    let mut i = 0;

    let mut val_stack = [0i64; 32];
    let mut val_top = 0;
    let mut op_stack = [0u8; 32];
    let mut op_top = 0;

    let mut expect_unary = true;

    while i < n {
        // Skip whitespace
        while i < n && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\r' || bytes[i] == b'\n') {
            i += 1;
        }
        if i >= n {
            break;
        }

        let b = bytes[i];
        if (b >= b'0' && b <= b'9') || (b == b'-' && expect_unary && i + 1 < n && bytes[i + 1] >= b'0' && bytes[i + 1] <= b'9') {
            let neg = b == b'-';
            if neg {
                i += 1;
            }
            let mut val = 0i64;
            while i < n && bytes[i] >= b'0' && bytes[i] <= b'9' {
                val = val.wrapping_mul(10).wrapping_add((bytes[i] - b'0') as i64);
                i += 1;
            }
            if neg {
                val = -val;
            }
            if val_top < val_stack.len() {
                val_stack[val_top] = val;
                val_top += 1;
            }
            expect_unary = false;
        } else if b == b'(' {
            if op_top < op_stack.len() {
                op_stack[op_top] = b;
                op_top += 1;
            }
            expect_unary = true;
            i += 1;
        } else if b == b')' {
            while op_top > 0 && op_stack[op_top - 1] != b'(' {
                let op = op_stack[op_top - 1];
                op_top -= 1;
                if !apply_op(&mut val_stack, &mut val_top, op) {
                    return;
                }
            }
            if op_top > 0 && op_stack[op_top - 1] == b'(' {
                op_top -= 1;
            }
            expect_unary = false;
            i += 1;
        } else if b == b'+' || b == b'-' || b == b'*' || b == b'/' || b == b'%' || b == b'^' {
            let prec = precedence(b);
            while op_top > 0 && op_stack[op_top - 1] != b'(' && precedence(op_stack[op_top - 1]) >= prec {
                let op = op_stack[op_top - 1];
                op_top -= 1;
                if !apply_op(&mut val_stack, &mut val_top, op) {
                    return;
                }
            }
            if op_top < op_stack.len() {
                op_stack[op_top] = b;
                op_top += 1;
            }
            expect_unary = true;
            i += 1;
        } else {
            i += 1;
        }
    }

    while op_top > 0 {
        let op = op_stack[op_top - 1];
        op_top -= 1;
        if op == b'(' || op == b')' {
            continue;
        }
        if !apply_op(&mut val_stack, &mut val_top, op) {
            return;
        }
    }

    if val_top > 0 {
        print_num(val_stack[val_top - 1]);
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut i = 1;
    let mut has_expr = false;

    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--help" || a == "-h" {
            println!("Usage: bc [OPTION] [EXPRESSION...]");
            println!("An arbitrary-precision and standard infix calculator language");
            println!("  -e, --expression=EXPR  evaluate EXPR string");
            return 0;
        } else if (a == "-e" || a == "--expression") && i + 1 < argc {
            i += 1;
            let expr = unsafe { arg(argv, i) };
            eval_expr(expr);
            has_expr = true;
        } else if !a.starts_with('-') {
            eval_expr(a);
            has_expr = true;
        }
        i += 1;
    }

    if !has_expr {
        let mut buf = [0u8; 256];
        let mut line_buf = [0u8; 128];
        let mut line_len = 0;

        loop {
            let n = read(0, &mut buf);
            if n <= 0 {
                break;
            }
            for &b in &buf[..n as usize] {
                if b == b'\n' {
                    if let Ok(s) = core::str::from_utf8(&line_buf[..line_len]) {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            eval_expr(trimmed);
                        }
                    }
                    line_len = 0;
                } else if line_len < line_buf.len() {
                    line_buf[line_len] = b;
                    line_len += 1;
                }
            }
        }
        if line_len > 0 {
            if let Ok(s) = core::str::from_utf8(&line_buf[..line_len]) {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    eval_expr(trimmed);
                }
            }
        }
    }

    0
}
