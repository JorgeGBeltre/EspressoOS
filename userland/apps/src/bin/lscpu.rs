#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, O_RDONLY};

fn read_proc<'a>(path: &str, buf: &'a mut [u8]) -> &'a str {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        return "";
    }
    let n = read(fd as i32, buf);
    close(fd as i32);
    if n > 0 {
        core::str::from_utf8(&buf[..n as usize]).unwrap_or("")
    } else {
        ""
    }
}

fn find_field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    for line in text.lines() {
        if let Some(idx) = line.find(':') {
            let k = line[..idx].trim();
            if k.eq_ignore_ascii_case(key) {
                return Some(line[idx + 1..].trim());
            }
        }
    }
    None
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let a = unsafe { arg(argv, 1) };
        if a == "-h" || a == "--help" {
            println!("Usage: lscpu");
            println!("Display live CPU, SMP, memory, and PMS architecture details from /proc and /sys");
            return 0;
        }
    }

    let mut cpu_buf = [0u8; 512];
    let mut mem_buf = [0u8; 512];
    let mut smp_buf = [0u8; 256];
    let mut pms_buf = [0u8; 256];

    let cpu = read_proc("/proc/cpuinfo", &mut cpu_buf);
    let mem = read_proc("/proc/meminfo", &mut mem_buf);
    let smp = read_proc("/sys/smp", &mut smp_buf);
    let pms = read_proc("/sys/pms", &mut pms_buf);

    let mut cores = 0usize;
    for line in cpu.lines() {
        if line.trim_start().starts_with("processor") {
            cores += 1;
        }
    }
    if cores == 0 {
        cores = 1;
    }

    let model = find_field(cpu, "model name").unwrap_or("Xtensa LX7");
    let arch = find_field(cpu, "cpu family").unwrap_or("xtensa");
    let mhz = find_field(cpu, "cpu MHz").unwrap_or("240");
    let bogomips = find_field(cpu, "bogomips").unwrap_or("240.00");
    let features = find_field(cpu, "Features")
        .or_else(|| find_field(cpu, "flags"))
        .unwrap_or("windowed, loop, mac16, mul32, s32c1i, fp");
    let mem_total = find_field(mem, "MemTotal").unwrap_or("unknown");
    let mem_free = find_field(mem, "MemFree").unwrap_or("unknown");
    let smp_line = smp.lines().next().unwrap_or("uniprocessor");
    let pms_line = pms.lines().next().unwrap_or("disabled");

    println!("Architecture:          {}", arch);
    println!("CPU op-mode(s):        32-bit (NOMMU PSRAM-Exec)");
    println!("Byte Order:            Little Endian");
    println!("CPU(s):                {}", cores);
    if cores > 1 {
        println!("On-line CPU(s) list:   0-{}", cores - 1);
    } else {
        println!("On-line CPU(s) list:   0");
    }
    println!("Model name:            {}", model);
    println!("CPU MHz:               {}", mhz);
    println!("BogoMIPS:              {}", bogomips);
    println!("Flags:                 {}", features);
    println!("SMP state (/sys/smp):  {}", smp_line.trim());
    println!("PMS state (/sys/pms):  {}", pms_line.trim());
    println!("Memory Total:          {}", mem_total);
    println!("Memory Free:           {}", mem_free);
    0
}
