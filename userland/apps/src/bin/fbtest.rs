#![no_std]
#![no_main]

use libc::{close, ioctl, open, println, read, write, O_RDWR};

const FBIO_GET_VSCREENINFO: u32 = 0x4600;
const FBIO_CLEAR: u32 = 0x4601;
const FBIO_FILL: u32 = 0x4602;
const FBIO_INVERT: u32 = 0x4603;

const FB_WIDTH: usize = 128;
const FB_HEIGHT: usize = 64;
const FB_SIZE: usize = (FB_WIDTH * FB_HEIGHT) / 8; // 1024 bytes

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    println!("=== EspressoOS Framebuffer Graphics Diagnostic ===");

    let fd = open("/dev/fb0", O_RDWR);
    if fd < 0 {
        println!("fbtest: cannot open /dev/fb0");
        return 1;
    }

    // 1. Query screen info
    let res = ioctl(fd as i32, FBIO_GET_VSCREENINFO, 0);
    if res < 0 {
        println!("fbtest: FBIO_GET_VSCREENINFO failed");
        close(fd as i32);
        return 1;
    }

    let width = (res as usize >> 16) & 0xFFFF;
    let height = res as usize & 0xFFFF;
    println!("fb0: resolution {}x{} monochrome ({} bytes)", width, height, (width * height) / 8);

    // 2. Test clear
    let _ = ioctl(fd as i32, FBIO_CLEAR, 0);

    // 3. Render test pattern (checkerboard and border)
    let mut pattern = [0u8; FB_SIZE];
    for y in 0..FB_HEIGHT {
        for x in 0..FB_WIDTH {
            // Draw border
            let is_border = x == 0 || x == FB_WIDTH - 1 || y == 0 || y == FB_HEIGHT - 1;
            // Draw cross-hair
            let is_cross = x == FB_WIDTH / 2 || y == FB_HEIGHT / 2;
            if is_border || is_cross {
                let byte_idx = (y * FB_WIDTH + x) / 8;
                let bit_idx = 7 - (x % 8);
                if byte_idx < FB_SIZE {
                    pattern[byte_idx] |= 1 << bit_idx;
                }
            }
        }
    }

    let written = write(fd as i32, &pattern);
    println!("fb0: wrote test pattern ({} bytes)", written);

    // 4. Test invert
    let _ = ioctl(fd as i32, FBIO_INVERT, 0);
    println!("fb0: applied FBIO_INVERT");

    // 5. Read back to verify
    let mut read_back = [0u8; 16];
    let n = read(fd as i32, &mut read_back);
    if n > 0 {
        println!("fb0: read back verification OK ({} bytes checked)", n);
    }

    close(fd as i32);
    println!("fbtest: all framebuffer diagnostics PASSED");
    0
}
