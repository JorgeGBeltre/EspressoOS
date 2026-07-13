#![allow(dead_code)]

use crate::prelude::*;
use crate::arch::xtensa::sync::Mutex;
use esp_wifi::ble::controller::BleConnector;

static ADVERTISING: Mutex<bool> = Mutex::new(false);
static CONNECTOR: Mutex<Option<BleConnector<'static>>> = Mutex::new(None);

pub fn init(bt_periph: esp_hal::peripherals::BT, init_ref: &'static esp_wifi::EspWifiController<'static>) {
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
            esp_println::println!("[ble] ERROR: controlador BLE no inicializado");
            return;
        }
    };
    
    use embedded_io::Write;
    
    // 1. Configurar Parámetros de Publicidad (HCI LE Set Advertising Parameters)
    let params: [u8; 19] = [
        0x01, // HCI Command Indicator
        0x06, 0x20, // Opcode: 0x2006
        0x0f, // Parameter Length: 15
        0x00, 0x08, // Interval Min: 0x0800 (1.28s)
        0x00, 0x08, // Interval Max: 0x0800
        0x00, // Adv Type: Connectable Undirected
        0x00, // Own Addr Type: Public
        0x00, // Peer Addr Type: Public
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Peer Addr
        0x07, // Channel Map: All channels
        0x00, // Filter Policy: Allow all
    ];
    let _ = conn.write(&params);
    
    // 2. Configurar Datos de Publicidad (HCI LE Set Advertising Data)
    let mut data: [u8; 36] = [0; 36];
    data[0] = 0x01; // HCI Command Indicator
    data[1] = 0x08; data[2] = 0x20; // Opcode: 0x2008
    data[3] = 32; // Parameter Length: 32 (1 + 31)
    data[4] = 14; // Advertising Data Length (Flags: 3 bytes + Name: 11 bytes)
    
    // AD Structure 1: Flags (Len: 2, Type: 0x01, Flags: 0x06)
    data[5] = 2;
    data[6] = 0x01;
    data[7] = 0x06;
    
    // AD Structure 2: Complete Local Name (Len: 11, Type: 0x09, Name: "EspressoOS")
    data[8] = 11;
    data[9] = 0x09;
    let name = b"EspressoOS";
    data[10..20].copy_from_slice(name);
    
    let _ = conn.write(&data);
    
    // 3. Habilitar Publicidad (HCI LE Set Advertising Enable)
    let enable: [u8; 5] = [
        0x01, // HCI Command Indicator
        0x0a, 0x20, // Opcode: 0x200a
        0x01, // Parameter Length: 1
        0x01, // Enable: 1
    ];
    let _ = conn.write(&enable);
    
    *adv = true;
    esp_println::println!("[ble] Publicidad BLE iniciada como 'EspressoOS'");
}

pub fn is_advertising() -> bool {
    *ADVERTISING.lock()
}
