#![no_std]
#![no_main]

use libc::{arg, println, print};

const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];

const DAYS_IN_MONTH: [u32; 12] = [
    31, 28, 31, 30, 31, 30,
    31, 31, 30, 31, 30, 31,
];

fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// Sakamoto's algorithm: returns day of week (0 = Sunday, 1 = Monday, ..., 6 = Saturday)
fn day_of_week(year: u32, month: u32, day: u32) -> u32 {
    let t: [u32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    (y + y / 4 - y / 100 + y / 400 + t[(month - 1) as usize] + day) % 7
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut val = 0u32;
    if s.is_empty() {
        return None;
    }
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}

fn print_calendar(month: u32, year: u32) {
    if month < 1 || month > 12 || year < 1 {
        println!("cal: illegal month or year");
        return;
    }

    let m_name = MONTH_NAMES[(month - 1) as usize];
    println!("    {:>9} {}", m_name, year);
    println!("Su Mo Tu We Th Fr Sa");

    let first_day = day_of_week(year, month, 1);
    let mut days = DAYS_IN_MONTH[(month - 1) as usize];
    if month == 2 && is_leap_year(year) {
        days = 29;
    }

    // Leading spaces for first week
    for _ in 0..first_day {
        print!("   ");
    }

    let mut current_day_col = first_day;
    for day in 1..=days {
        if day < 10 {
            print!(" {}", day);
        } else {
            print!("{}", day);
        }

        current_day_col += 1;
        if current_day_col == 7 {
            println!();
            current_day_col = 0;
        } else if day != days {
            print!(" ");
        }
    }

    if current_day_col != 0 {
        println!();
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut month = 10u32; // Default: October
    let mut year = 2026u32; // Default: 2026

    if argc == 2 {
        let first = unsafe { arg(argv, 1) };
        if first == "-h" || first == "--help" || first == "help" {
            println!("Usage: cal [month] [year]");
            println!("Display a calendar");
            println!("  cal               Current month (October 2026)");
            println!("  cal <year>        Display entire year");
            println!("  cal <month> <yr>  Specific month (1-12) and year");
            return 0;
        }
        if let Some(y) = parse_u32(first) {
            if y > 12 {
                // Year requested: print 12 months
                for m in 1..=12 {
                    print_calendar(m, y);
                    println!();
                }
                return 0;
            } else {
                month = y;
            }
        }
    } else if argc >= 3 {
        let m_str = unsafe { arg(argv, 1) };
        let y_str = unsafe { arg(argv, 2) };
        if let (Some(m), Some(y)) = (parse_u32(m_str), parse_u32(y_str)) {
            month = m;
            year = y;
        } else {
            println!("cal: invalid argument");
            return 1;
        }
    }

    print_calendar(month, year);
    0
}
