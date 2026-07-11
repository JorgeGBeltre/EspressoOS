//! Protección de memoria del kernel (PMS / World Controller) — Fase 8.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! El ESP32-S3 NO tiene MMU de paginación de propósito general (sin traducción
//! de direcciones virtuales), pero sí un controlador de permisos de memoria
//! (PMS, "Permission Control", integrado con el World Controller) capaz de
//! restringir acceso de lectura/escritura/ejecución a rangos de memoria por
//! "mundo" (kernel vs. tarea). Se usa en Fase 8 para aislar código y estructuras
//! críticas del kernel de las tareas de usuario.
//!
//! ESTADO: esqueleto documentado. La programación real de los registros PMS es
//! HARDWARE específico y arriesgado (configurar mal puede colgar el arranque),
//! por eso se deja como no-op seguro hasta Fase 8. No panica.

#![allow(dead_code)]

/// Configura las regiones protegidas del kernel vía PMS. Fase 8. [CANÓNICO]
///
/// Idempotente y no-op por ahora: no toca hardware hasta implementar Fase 8.
///
/// Plan de implementación (Fase 8), a modo de guía para quien lo aborde:
///  1. Definir los rangos a proteger a partir de símbolos del linker
///     (`_stext`/`_etext` para código, la región del heap/pilas del kernel).
///  2. Programar los registros PMS del periférico correspondiente
///     (bloques `SENSITIVE` / World Controller del S3) para conceder al "mundo"
///     de las tareas solo lectura sobre el código y denegar acceso a las
///     estructuras del kernel. INCIERTO: la superficie de estos registros no está
///     expuesta de forma estable por `esp-hal` 0.23; probablemente haya que
///     escribir MMIO crudo con offsets del TRM (Technical Reference Manual).
///  3. Instalar el manejador de excepción de acceso (fault) que reporte la
///     violación como `KError::Fault` hacia el subsistema que corresponda.
///
/// Mientras no se implemente, dejar como no-op EVITA bloquear el arranque de las
/// fases anteriores, que no dependen de la protección.
pub fn init() {
    // TODO(fase-8): programar PMS/World Controller para las regiones críticas.
    // No-op seguro hasta entonces.
}
