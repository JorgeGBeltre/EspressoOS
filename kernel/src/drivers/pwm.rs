#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;
use core::sync::atomic::{AtomicBool, Ordering};

const LEDC_BASE: usize = 0x6001_9000;
const LEDC_CONF_REG: usize = LEDC_BASE + 0x00D0;

const LEDC_CH_CONF0_BASE: usize = LEDC_BASE + 0x0000;
const LEDC_CH_DUTY_BASE: usize = LEDC_BASE + 0x0004;
const LEDC_CH_CONF1_BASE: usize = LEDC_BASE + 0x000C;

const LEDC_TIMER0_CONF_REG: usize = LEDC_BASE + 0x00A0;

const GPIO_BASE: usize = 0x6000_4000;
const GPIO_ENABLE_W1TS: usize = GPIO_BASE + 0x0024;
const GPIO_ENABLE_W1TC: usize = GPIO_BASE + 0x0028;
const GPIO_ENABLE1_W1TS: usize = GPIO_BASE + 0x0030;
const GPIO_ENABLE1_W1TC: usize = GPIO_BASE + 0x0034;
const GPIO_FUNC_OUT_SEL_BASE: usize = GPIO_BASE + 0x0554;

const LEDC_LS_SIG_OUT0_IDX: u32 = 45;
const SIG_GPIO_OUT_IDX: u32 = 128;
const MAX_CHANNELS: usize = 8;
const MAX_PINS: u8 = 48;
const APB_CLK_HZ: u32 = 80_000_000;

#[inline(always)]
unsafe fn reg_read(addr: usize) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline(always)]
unsafe fn reg_write(addr: usize, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val)
}

#[derive(Clone, Copy)]
pub struct PwmChannelConfig {
    pub pin: u8,
    pub freq_hz: u32,
    pub duty_percent: u8,
    pub enabled: bool,
}

impl Default for PwmChannelConfig {
    fn default() -> Self {
        Self {
            pin: 0xFF,
            freq_hz: 1000,
            duty_percent: 0,
            enabled: false,
        }
    }
}

static PWM_INITIALIZED: AtomicBool = AtomicBool::new(false);
static CHANNELS: Mutex<[PwmChannelConfig; MAX_CHANNELS]> =
    Mutex::new([PwmChannelConfig {
        pin: 0xFF,
        freq_hz: 1000,
        duty_percent: 0,
        enabled: false,
    }; MAX_CHANNELS]);

pub fn init() {
    if PWM_INITIALIZED.swap(true, Ordering::SeqCst) {
        return;
    }
    // Enable APB clock for LEDC peripheral
    unsafe {
        reg_write(LEDC_CONF_REG, 1);
    }
}

fn set_gpio_matrix(pin: u8, sig_idx: u32, enable: bool) {
    if pin > MAX_PINS {
        return;
    }
    unsafe {
        reg_write(
            GPIO_FUNC_OUT_SEL_BASE + (pin as usize) * 4,
            sig_idx,
        );
        if pin < 32 {
            let reg = if enable { GPIO_ENABLE_W1TS } else { GPIO_ENABLE_W1TC };
            reg_write(reg, 1u32 << pin);
        } else {
            let reg = if enable { GPIO_ENABLE1_W1TS } else { GPIO_ENABLE1_W1TC };
            reg_write(reg, 1u32 << (pin - 32));
        }
    }
}

pub fn configure_channel(ch: usize, pin: u8, freq_hz: u32, duty_percent: u8) -> KResult<()> {
    if ch >= MAX_CHANNELS || pin > MAX_PINS || duty_percent > 100 {
        return Err(KError::InvalidArgument);
    }
    let freq = if freq_hz == 0 { 1000 } else { freq_hz.min(40_000_000) };
    init();

    // 10-bit resolution (1024 levels)
    let res_bits = 10u32;
    let max_val = 1u32 << res_bits;
    // Calculate clock divider: APB_CLK / (freq * 1024) * 256 (fixed point 10.8)
    let div_fp = ((APB_CLK_HZ as u64 * 256) / (freq as u64 * max_val as u64)) as u32;
    let div_clamped = div_fp.max(256).min((1 << 18) - 1);

    unsafe {
        // Configure Timer 0 (used by default for channels)
        // bit 0..3: duty_res, bit 4..21: clock_div, bit 24: tick_sel (1 = apb_clk)
        let timer_conf = (res_bits & 0x0F) | (div_clamped << 4) | (1 << 24);
        reg_write(LEDC_TIMER0_CONF_REG, timer_conf);

        // Configure Channel CONF0
        let conf0_addr = LEDC_CH_CONF0_BASE + ch * 0x14;
        // timer_sel = 0 (bits 0..1), sig_out_en = 1 (bit 2)
        reg_write(conf0_addr, 1 << 2);

        // Configure Duty: duty_percent * 1023 / 100, shifted by 4
        let duty_val = ((duty_percent as u32 * (max_val - 1)) / 100) << 4;
        let duty_addr = LEDC_CH_DUTY_BASE + ch * 0x14;
        reg_write(duty_addr, duty_val);

        // Latch duty in CONF1 (bit 31 = duty_start)
        let conf1_addr = LEDC_CH_CONF1_BASE + ch * 0x14;
        reg_write(conf1_addr, 1u32 << 31);
    }

    // Connect channel output to GPIO pin
    set_gpio_matrix(pin, LEDC_LS_SIG_OUT0_IDX + ch as u32, true);

    let mut guard = CHANNELS.lock();
    guard[ch] = PwmChannelConfig {
        pin,
        freq_hz: freq,
        duty_percent,
        enabled: true,
    };
    Ok(())
}

pub fn set_duty(ch: usize, duty_percent: u8) -> KResult<()> {
    if ch >= MAX_CHANNELS || duty_percent > 100 {
        return Err(KError::InvalidArgument);
    }
    let mut guard = CHANNELS.lock();
    if !guard[ch].enabled {
        return Err(KError::PermissionDenied);
    }
    let max_val = 1024u32;
    let duty_val = ((duty_percent as u32 * (max_val - 1)) / 100) << 4;

    unsafe {
        let duty_addr = LEDC_CH_DUTY_BASE + ch * 0x14;
        reg_write(duty_addr, duty_val);

        let conf1_addr = LEDC_CH_CONF1_BASE + ch * 0x14;
        reg_write(conf1_addr, 1u32 << 31);
    }

    guard[ch].duty_percent = duty_percent;
    Ok(())
}

pub fn disable_channel(ch: usize) -> KResult<()> {
    if ch >= MAX_CHANNELS {
        return Err(KError::InvalidArgument);
    }
    let mut guard = CHANNELS.lock();
    if guard[ch].enabled {
        let pin = guard[ch].pin;
        unsafe {
            let duty_addr = LEDC_CH_DUTY_BASE + ch * 0x14;
            reg_write(duty_addr, 0);
            let conf1_addr = LEDC_CH_CONF1_BASE + ch * 0x14;
            reg_write(conf1_addr, 1u32 << 31);
        }
        set_gpio_matrix(pin, SIG_GPIO_OUT_IDX, false);
        guard[ch].enabled = false;
    }
    Ok(())
}

pub const PWM_IOC_CONFIG: u32 = 0x5001;
pub const PWM_IOC_SET_DUTY: u32 = 0x5002;
pub const PWM_IOC_ENABLE: u32 = 0x5003;
pub const PWM_IOC_DISABLE: u32 = 0x5004;
pub const PWM_IOC_GET_STATUS: u32 = 0x5005;

pub struct PwmDevice;

impl crate::vfs::devfs::Device for PwmDevice {
    fn read(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let guard = CHANNELS.lock();
        let mut s = alloc::string::String::from("CH  PIN  FREQ(Hz)  DUTY(%)  STATUS\n");
        for (i, ch) in guard.iter().enumerate() {
            let status = if ch.enabled { "ENABLED" } else { "DISABLED" };
            let pin_str = if ch.pin <= MAX_PINS {
                alloc::format!("{:3}", ch.pin)
            } else {
                alloc::string::String::from(" --")
            };
            s.push_str(&alloc::format!(
                "{:2}  {}  {:8}  {:7}  {}\n",
                i, pin_str, ch.freq_hz, ch.duty_percent, status
            ));
        }

        let bytes = s.as_bytes();
        let off = off as usize;
        if off >= bytes.len() {
            return Ok(0);
        }
        let avail = &bytes[off..];
        let n = buf.len().min(avail.len());
        buf[..n].copy_from_slice(&avail[..n]);
        Ok(n)
    }

    fn write(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        if let Ok(cmd_str) = core::str::from_utf8(buf) {
            let trimmed = cmd_str.trim();
            let mut parts = trimmed.split_whitespace();
            if let Some(verb) = parts.next() {
                match verb {
                    "config" => {
                        // config <ch> <pin> <freq> <duty>
                        if let (Some(ch_s), Some(pin_s), Some(freq_s), Some(duty_s)) =
                            (parts.next(), parts.next(), parts.next(), parts.next())
                        {
                            if let (Ok(ch), Ok(pin), Ok(freq), Ok(duty)) = (
                                ch_s.parse::<usize>(),
                                pin_s.parse::<u8>(),
                                freq_s.parse::<u32>(),
                                duty_s.parse::<u8>(),
                            ) {
                                configure_channel(ch, pin, freq, duty)?;
                                return Ok(buf.len());
                            }
                        }
                    }
                    "duty" => {
                        // duty <ch> <duty>
                        if let (Some(ch_s), Some(duty_s)) = (parts.next(), parts.next()) {
                            if let (Ok(ch), Ok(duty)) =
                                (ch_s.parse::<usize>(), duty_s.parse::<u8>())
                            {
                                set_duty(ch, duty)?;
                                return Ok(buf.len());
                            }
                        }
                    }
                    "disable" | "stop" => {
                        // disable <ch>
                        if let Some(ch_s) = parts.next() {
                            if let Ok(ch) = ch_s.parse::<usize>() {
                                disable_channel(ch)?;
                                return Ok(buf.len());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(buf.len())
    }

    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<usize> {
        match cmd {
            PWM_IOC_CONFIG => {
                // arg: (ch & 0xFF) | ((pin & 0xFF) << 8) | ((duty & 0xFF) << 16) | ((freq & 0xFF) << 24)
                let ch = arg & 0xFF;
                let pin = ((arg >> 8) & 0xFF) as u8;
                let duty = ((arg >> 16) & 0xFF) as u8;
                let freq = 1000u32;
                configure_channel(ch, pin, freq, duty)?;
                Ok(0)
            }
            PWM_IOC_SET_DUTY => {
                let ch = arg & 0xFF;
                let duty = ((arg >> 8) & 0xFF) as u8;
                set_duty(ch, duty)?;
                Ok(0)
            }
            PWM_IOC_ENABLE => {
                let ch = arg & 0xFF;
                let guard = CHANNELS.lock();
                if ch < MAX_CHANNELS && guard[ch].pin <= MAX_PINS {
                    let pin = guard[ch].pin;
                    let freq = guard[ch].freq_hz;
                    let duty = guard[ch].duty_percent;
                    drop(guard);
                    configure_channel(ch, pin, freq, duty)?;
                    Ok(0)
                } else {
                    Err(KError::InvalidArgument)
                }
            }
            PWM_IOC_DISABLE => {
                let ch = arg & 0xFF;
                disable_channel(ch)?;
                Ok(0)
            }
            PWM_IOC_GET_STATUS => {
                let ch = arg & 0xFF;
                if ch >= MAX_CHANNELS {
                    return Err(KError::InvalidArgument);
                }
                let guard = CHANNELS.lock();
                let status = if guard[ch].enabled { 1 } else { 0 };
                Ok(status | ((guard[ch].duty_percent as usize) << 8))
            }
            _ => Err(KError::InvalidArgument),
        }
    }
}

pub fn devfs_device() -> Arc<dyn crate::vfs::devfs::Device> {
    Arc::new(PwmDevice)
}
