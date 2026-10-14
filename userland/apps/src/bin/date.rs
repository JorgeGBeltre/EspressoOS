#![no_std]
#![no_main]

use libc::{gettimeofday, println, timeval, uptime_ms};

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut tv = timeval { tv_sec: 0, tv_usec: 0 };
    let res = gettimeofday(&mut tv);

    if res == 0 && tv.tv_sec >= 1577836800 {
        // Epoch seconds >= 2020-01-01
        let secs = tv.tv_sec as u64;
        let days = secs / 86400;
        let day_secs = secs % 86400;

        let hours = day_secs / 3600;
        let mins = (day_secs % 3600) / 60;
        let s = day_secs % 60;

        // 1970-01-01 was Thursday (index 4)
        let wday = ((days + 4) % 7) as usize;

        // Howard Hinnant's algorithm
        let z = days + 719468;
        let era = z / 146097;
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = y + if m <= 2 { 1 } else { 0 };

        let m_idx = (m - 1).min(11) as usize;
        println!(
            "{} {} {:02} {:02}:{:02}:{:02} UTC {}",
            DAYS[wday], MONTHS[m_idx], d, hours, mins, s, year
        );
    } else {
        let ms = uptime_ms();
        let sec = ms / 1000;
        let min = sec / 60;
        let hours = min / 60;
        let days = hours / 24;
        println!(
            "Thu Jan  1 00:00:{:02} UTC 1970 (clock unsynchronized, uptime: {}d {:02}:{:02}:{:02})",
            sec % 60, days, hours % 24, min % 60, sec % 60
        );
    }
    0
}
