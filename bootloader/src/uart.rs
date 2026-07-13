#![allow(dead_code)]

pub fn putc(_b: u8) {
    
}

pub fn puts(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}
