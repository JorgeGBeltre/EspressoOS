#![allow(dead_code)]

use crate::prelude::*;
use esp_hal::sha::{Sha, Sha256};
use nb::block;

/// Realiza un hash SHA-256 acelerado por hardware (usando el coprocesador SHA del ESP32-S3).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let sha_periph = unsafe { esp_hal::peripherals::SHA::steal() };
    let mut sha = Sha::new(sha_periph);
    let mut hasher = sha.start::<Sha256>();
    
    let mut remaining = data;
    while !remaining.is_empty() {
        remaining = block!(hasher.update(remaining)).unwrap();
    }
    
    let mut output = [0u8; 32];
    block!(hasher.finish(&mut output)).unwrap();
    output
}
