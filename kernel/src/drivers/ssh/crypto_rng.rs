#![allow(dead_code)]

use esp_hal::rng::Rng;
use rand_core::{CryptoRng, RngCore};

pub struct HwRng(pub Rng);

impl HwRng {
    pub fn new(rng: Rng) -> Self {
        Self(rng)
    }
}

impl RngCore for HwRng {
    fn next_u32(&mut self) -> u32 {
        self.0.random()
    }

    fn next_u64(&mut self) -> u64 {
        ((self.0.random() as u64) << 32) | (self.0.random() as u64)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.0.read(dest);
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for HwRng {}
