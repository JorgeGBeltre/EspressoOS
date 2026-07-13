#![allow(dead_code)]

//! Protección de memoria PMS / World-Controller del ESP32-S3 (Fase 8, feature `pms`).
//!
//! El ESP32-S3 divide la ejecución en dos "mundos" de privilegio (World-0 y
//! World-1) y permite fijar permisos de acceso por región vía el periférico
//! `SENSITIVE` (registros `*_pms_constrain_*`), además de un **monitor de
//! violaciones** que registra el mundo y la dirección de un acceso ilegal.
//!
//! ## Estrategia (segura por diseño)
//! El kernel se ejecuta en **World-0**. Por eso:
//! - En boot (con `--features pms`) sólo habilitamos el **monitor de violaciones**
//!   de DRAM0: observabilidad pura, NO restringe ningún acceso, así que no puede
//!   colgar el arranque. `report()` lo lee.
//! - Restringir **World-1** (`protect_world1`) es seguro para el kernel en marcha
//!   (nada corre aún en World-1) y es exactamente el cimiento de aislamiento para
//!   un futuro userland. Se aplica sólo bajo demanda (comando `pms world1`).
//!
//! ## Validación
//! El monitor/`report()` es verificable en hardware sin riesgo. El encoding exacto
//! de los campos de permiso debe confirmarse contra el TRM del ESP32-S3 antes de
//! confiar en `protect_world1` para un userland real. Ver
//! `docs/design/remaining-phases.md`.

/// Llamada en boot desde `main`. Sin `pms` es un no-op (imagen por defecto intacta).
pub fn init() {
    #[cfg(feature = "pms")]
    imp::init();
}

/// Reporte legible del estado PMS. `None` si se compiló sin `pms`.
pub fn report() -> Option<crate::prelude::String> {
    #[cfg(feature = "pms")]
    {
        Some(imp::report())
    }
    #[cfg(not(feature = "pms"))]
    {
        None
    }
}

/// Restringe el acceso de World-1 a la SRAM (experimental; el kernel es World-0,
/// así que no lo afecta). `None` si se compiló sin `pms`.
pub fn protect_world1() -> Option<crate::prelude::String> {
    #[cfg(feature = "pms")]
    {
        Some(imp::protect_world1())
    }
    #[cfg(not(feature = "pms"))]
    {
        None
    }
}

#[cfg(feature = "pms")]
mod imp {
    use crate::prelude::*;
    use esp_println::println;

    // El tipo del bloque de registros (`sensitive::RegisterBlock`) es privado en
    // esp-hal, así que no lo nombramos: `&*PTR` infiere el tipo. SAFETY: SENSITIVE
    // no lo posee ningún otro subsistema; sólo tocamos los registros PMS de DRAM0.
    macro_rules! sensitive {
        () => {
            unsafe { &*esp_hal::peripherals::SENSITIVE::PTR }
        };
    }

    pub fn init() {
        let s = sensitive!();
        // 1) Monitor de violaciones DRAM0 (observabilidad): registra el mundo y la
        //    dirección de cualquier acceso ilegal bajo la config vigente.
        s.core_0_dram0_pms_monitor_1().modify(|_, w| {
            w.core_0_dram0_pms_monitor_violate_clr().set_bit();
            w.core_0_dram0_pms_monitor_violate_en().set_bit()
        });
        // 2) Enforcement en boot: restringe (sin acceso) las 4 regiones de datos
        //    SRAM de World-1. Seguro porque el kernel corre en World-0; deja el
        //    aislamiento listo para un futuro userland (que correría en World-1).
        s.core_x_dram0_pms_constrain_1().modify(|_, w| unsafe {
            w.core_x_dram0_pms_constrain_sram_world_1_pms_0().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_1().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_2().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_3().bits(0)
        });
        println!("[pms] monitor DRAM0 + enforcement World-1 activos (kernel = World-0)");
    }

    pub fn report() -> String {
        let s = sensitive!();
        let m1 = s.core_0_dram0_pms_monitor_1().read();
        let en = m1.core_0_dram0_pms_monitor_violate_en().bit_is_set();
        let m2 = s.core_0_dram0_pms_monitor_2().read();
        let intr = m2.core_0_dram0_pms_monitor_violate_intr().bit_is_set();
        let world = m2.core_0_dram0_pms_monitor_violate_status_world().bits();
        let addr = m2.core_0_dram0_pms_monitor_violate_status_addr().bits();
        let constrain1 = s.core_x_dram0_pms_constrain_1().read().bits();
        alloc::format!(
            "PMS DRAM0: monitor_en={} violacion={} world={} addr_field={:#08x} constrain_1={:#010x}",
            en, intr, world, addr, constrain1
        )
    }

    pub fn protect_world1() -> String {
        let s = sensitive!();
        // Poner a 0 (sin acceso) las 4 regiones de datos SRAM de World-1. El
        // kernel corre en World-0, cuyos campos NO se tocan. `bits()` es unsafe
        // (acepta cualquier valor de campo); 0 es válido.
        s.core_x_dram0_pms_constrain_1().modify(|_, w| unsafe {
            w.core_x_dram0_pms_constrain_sram_world_1_pms_0().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_1().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_2().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_3().bits(0)
        });
        let constrain1 = s.core_x_dram0_pms_constrain_1().read().bits();
        println!("[pms] World-1 SRAM restringido (kernel World-0 intacto)");
        alloc::format!(
            "World-1 SRAM (4 regiones) -> sin acceso; constrain_1={:#010x}",
            constrain1
        )
    }
}
