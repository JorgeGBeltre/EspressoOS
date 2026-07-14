#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;
use esp_wifi::ble::controller::BleConnector;

static ADVERTISING: Mutex<bool> = Mutex::new(false);
static CONNECTOR: Mutex<Option<BleConnector<'static>>> = Mutex::new(None);

pub fn init(
    bt_periph: esp_hal::peripherals::BT,
    init_ref: &'static esp_wifi::EspWifiController<'static>,
) {
    let conn = BleConnector::new(init_ref, bt_periph);
    crate::arch::xtensa::interrupts::critical_section(|| {
        *CONNECTOR.lock() = Some(conn);
    });
}

pub fn start_advertising() {
    let mut adv = ADVERTISING.lock();
    if *adv {
        return;
    }

    let mut conn_guard = CONNECTOR.lock();
    let conn = match conn_guard.as_mut() {
        Some(c) => c,
        None => {
            esp_println::println!("[ble] ERROR: BLE controller not initialized");
            return;
        }
    };

    use embedded_io::Write;

    let params: [u8; 19] = [
        0x01, 0x06, 0x20, 0x0f, 0x00, 0x08, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x07, 0x00,
    ];
    let _ = conn.write(&params);

    let mut data: [u8; 36] = [0; 36];
    data[0] = 0x01;
    data[1] = 0x08;
    data[2] = 0x20;
    data[3] = 32;
    data[4] = 14;

    data[5] = 2;
    data[6] = 0x01;
    data[7] = 0x06;

    data[8] = 11;
    data[9] = 0x09;
    let name = b"EspressoOS";
    data[10..20].copy_from_slice(name);

    let _ = conn.write(&data);

    let enable: [u8; 5] = [0x01, 0x0a, 0x20, 0x01, 0x01];
    let _ = conn.write(&enable);

    *adv = true;
    esp_println::println!("[ble] BLE advertising started as 'EspressoOS'");
}

pub fn is_advertising() -> bool {
    *ADVERTISING.lock()
}
