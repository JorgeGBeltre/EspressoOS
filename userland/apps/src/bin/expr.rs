#![no_std]
#![no_main]

use libc::{arg, println};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("expr: syntax error: missing operand");
        return 2;
    }

    if argc == 3 {
        let op = unsafe { arg(argv, 1) };
        let s = unsafe { arg(argv, 2) };
        if op == "length" {
            println!("{}", s.len());
            return 0;
        }
    }

    if argc == 4 {
        let a_str = unsafe { arg(argv, 1) };
        let op = unsafe { arg(argv, 2) };
        let b_str = unsafe { arg(argv, 3) };

        let a = match a_str.parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                println!("expr: non-integer argument: {}", a_str);
                return 2;
            }
        };
        let b = match b_str.parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                println!("expr: non-integer argument: {}", b_str);
                return 2;
            }
        };

        let result = match op {
            "+" => a.saturating_add(b),
            "-" => a.saturating_sub(b),
            "*" | "x" => a.saturating_mul(b),
            "/" => {
                if b == 0 {
                    println!("expr: division by zero");
                    return 2;
                }
                a / b
            }
            "%" => {
                if b == 0 {
                    println!("expr: division by zero");
                    return 2;
                }
                a % b
            }
            "=" => if a == b { 1 } else { 0 },
            "!=" => if a != b { 1 } else { 0 },
            "<" => if a < b { 1 } else { 0 },
            "<=" => if a <= b { 1 } else { 0 },
            ">" => if a > b { 1 } else { 0 },
            ">=" => if a >= b { 1 } else { 0 },
            _ => {
                println!("expr: syntax error: unknown operator {}", op);
                return 2;
            }
        };

        println!("{}", result);
        return if result == 0 { 1 } else { 0 };
    }

    println!("usage: expr <num1> <op> <num2>");
    println!("       expr length <string>");
    2
}
