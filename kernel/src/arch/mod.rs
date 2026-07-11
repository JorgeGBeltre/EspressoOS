//! Capa dependiente de arquitectura.
//!
//! Aísla todo lo específico del Xtensa LX7 (cambio de contexto, vectores de
//! excepción, temporizador, primitivas atómicas) para que el resto del kernel
//! sea, en lo posible, agnóstico al hardware.
pub mod xtensa;
