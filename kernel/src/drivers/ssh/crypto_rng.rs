//! Adaptador del TRNG del ESP32-S3 -> `rand_core 0.6` (`RngCore` + `CryptoRng`).
// COMPILE-STATUS: borrador
//!
//! Es la ÚNICA fuente de entropía del subsistema SSH. Envuelve
//! `esp_hal::rng::Rng` (el generador hardware del ESP32-S3) y lo expone con los
//! traits que exigen `x25519-dalek`/`ed25519-dalek` para generar claves efímeras
//! y semillas (`EphemeralSecret::random_from_rng`, etc.).
//!
//! Se define un adaptador PROPIO contra `rand_core 0.6` (la versión que usan las
//! dalek de este árbol) en lugar de reusar el `impl RngCore` que trae esp-hal:
//! así evitamos cualquier *skew* entre la `rand_core` que reexporta esp-hal
//! (podría ser 0.9) y la 0.6 que necesitan las crates de firma/ECDH.
//!
//! SEGURIDAD: `esp_hal::rng::Rng` solo garantiza entropía real de calidad
//! criptográfica con la radio (WiFi/BT) o el ADC activos. En `net_task` la radio
//! está encendida, así que ahí es un TRNG válido. La implementación de
//! `CryptoRng` de abajo es esa ASERCIÓN: NO instancies `HwRng` fuera de un
//! contexto con la radio activa.
#![allow(dead_code)]

use esp_hal::rng::Rng;
use rand_core::{CryptoRng, RngCore};

/// Envuelve el TRNG hardware del ESP32-S3 como CSPRNG para `rand_core 0.6`.
///
/// `Rng` es `Copy`, así que puede clonarse antes de moverlo a `esp_wifi::init`
/// (que lo consume por valor) para conservar una copia con la que sembrar SSH.
pub struct HwRng(pub Rng);

impl HwRng {
    /// Construye el adaptador a partir del `Rng` del HAL.
    pub fn new(rng: Rng) -> Self {
        Self(rng)
    }
}

impl RngCore for HwRng {
    fn next_u32(&mut self) -> u32 {
        self.0.random()
    }

    fn next_u64(&mut self) -> u64 {
        // Dos lecturas de 32 bits del TRNG concatenadas en big-endian lógico.
        ((self.0.random() as u64) << 32) | (self.0.random() as u64)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        // `Rng::read` rellena el búfer completo desde el TRNG (trozos de 4 bytes).
        self.0.read(dest);
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

/// ASERCIÓN de calidad criptográfica: solo válida con la radio activa
/// (`net_task`). `x25519-dalek`/`ed25519-dalek` exigen este marcador para las
/// rutas `random_from_rng`.
impl CryptoRng for HwRng {}
