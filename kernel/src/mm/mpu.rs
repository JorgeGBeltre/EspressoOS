#![allow(dead_code)]

pub fn init() {
    #[cfg(feature = "pms")]
    imp::init();
}

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

pub fn protect_world1_wx() -> Option<crate::prelude::String> {
    #[cfg(feature = "pms")]
    {
        Some(imp::protect_world1_wx())
    }
    #[cfg(not(feature = "pms"))]
    {
        None
    }
}

pub fn configure_stack_guard(core: usize, sp_min: u32, sp_max: u32) {
    #[cfg(feature = "pms")]
    imp::configure_stack_guard(core, sp_min, sp_max);
}

pub fn prepare_world_switch(is_user: bool, next_sp: u32) {
    #[cfg(feature = "pms")]
    imp::prepare_world_switch(is_user, next_sp);
}

#[cfg(feature = "pms")]
mod imp {
    use crate::prelude::*;
    use esp_println::println;

    macro_rules! sensitive {
        () => {
            unsafe { &*esp_hal::peripherals::SENSITIVE::PTR }
        };
    }

    pub fn init() {
        let s = sensitive!();

        s.core_0_dram0_pms_monitor_1().modify(|_, w| {
            w.core_0_dram0_pms_monitor_violate_clr().set_bit();
            w.core_0_dram0_pms_monitor_violate_en().set_bit()
        });

        s.core_x_dram0_pms_constrain_1().modify(|_, w| unsafe {
            w.core_x_dram0_pms_constrain_sram_world_1_pms_0().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_1().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_2().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_3().bits(0)
        });

        s.core_x_iram0_pms_constrain_1().modify(|_, w| unsafe {
            w.core_x_iram0_pms_constrain_sram_world_1_pms_0().bits(0);
            w.core_x_iram0_pms_constrain_sram_world_1_pms_1().bits(0);
            w.core_x_iram0_pms_constrain_sram_world_1_pms_2().bits(0);
            w.core_x_iram0_pms_constrain_sram_world_1_pms_3().bits(0)
        });

        println!("[pms] DRAM0 monitor + World-1 enforcement (DRAM/IRAM) active (kernel = World-0)");
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
            "PMS DRAM0: monitor_en={} violation={} world={} addr_field={:#08x} constrain_1={:#010x}",
            en, intr, world, addr, constrain1
        )
    }

    pub fn protect_world1() -> String {
        let s = sensitive!();
        s.core_x_dram0_pms_constrain_1().modify(|_, w| unsafe {
            w.core_x_dram0_pms_constrain_sram_world_1_pms_0().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_1().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_2().bits(0);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_3().bits(0)
        });
        let constrain1 = s.core_x_dram0_pms_constrain_1().read().bits();
        println!("[pms] World-1 SRAM locked");
        alloc::format!(
            "World-1 SRAM -> no access; constrain_1={:#010x}",
            constrain1
        )
    }

    pub fn protect_world1_wx() -> String {
        let s = sensitive!();

        s.core_x_dram0_pms_constrain_1().modify(|_, w| unsafe {
            w.core_x_dram0_pms_constrain_sram_world_1_pms_0().bits(3);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_1().bits(3);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_2().bits(3);
            w.core_x_dram0_pms_constrain_sram_world_1_pms_3().bits(3)
        });

        s.core_x_iram0_pms_constrain_1().modify(|_, w| unsafe {
            w.core_x_iram0_pms_constrain_sram_world_1_pms_0().bits(5);
            w.core_x_iram0_pms_constrain_sram_world_1_pms_1().bits(5);
            w.core_x_iram0_pms_constrain_sram_world_1_pms_2().bits(5);
            w.core_x_iram0_pms_constrain_sram_world_1_pms_3().bits(5)
        });
        let dram_val = s.core_x_dram0_pms_constrain_1().read().bits();
        let iram_val = s.core_x_iram0_pms_constrain_1().read().bits();
        println!("[pms] World-1 W^X enabled: DRAM0 (R+W), IRAM0 (R+X)");
        alloc::format!("DRAM0 R+W={:#x}, IRAM0 R+X={:#x}", dram_val, iram_val)
    }

    pub fn configure_stack_guard(core: usize, sp_min: u32, sp_max: u32) {
        let debug = unsafe { &*esp_hal::peripherals::ASSIST_DEBUG::PTR };
        if core == 1 {
            debug.core_1_montr_ena().modify(|_, w| {
                w.core_1_sp_spill_min_ena().clear_bit();
                w.core_1_sp_spill_max_ena().clear_bit()
            });
            debug.core_1_sp_min().write(|w| unsafe { w.bits(sp_min) });
            debug.core_1_sp_max().write(|w| unsafe { w.bits(sp_max) });
            debug.core_1_montr_ena().modify(|_, w| {
                w.core_1_sp_spill_min_ena().set_bit();
                w.core_1_sp_spill_max_ena().set_bit()
            });
        } else {
            debug.core_0_montr_ena().modify(|_, w| {
                w.core_0_sp_spill_min_ena().clear_bit();
                w.core_0_sp_spill_max_ena().clear_bit()
            });
            debug.core_0_sp_min().write(|w| unsafe { w.bits(sp_min) });
            debug.core_0_sp_max().write(|w| unsafe { w.bits(sp_max) });
            debug.core_0_montr_ena().modify(|_, w| {
                w.core_0_sp_spill_min_ena().set_bit();
                w.core_0_sp_spill_max_ena().set_bit()
            });
        }
    }

    pub fn prepare_world_switch(is_user: bool, next_sp: u32) {
        let wcl = unsafe { &*esp_hal::peripherals::WCL::PTR };
        if is_user {
            let next_pc = unsafe { *(next_sp as *const u32) };
            wcl.core_0_world_prepare()
                .write(|w| unsafe { w.core_0_world_prepare().bits(1) });
            wcl.core_0_world_trigger_addr()
                .write(|w| unsafe { w.core_0_world_trigger_addr().bits(next_pc) });
            wcl.core_0_world_update()
                .write(|w| unsafe { w.core_0_update().bits(1) });
        } else {
            wcl.core_0_world_prepare()
                .write(|w| unsafe { w.core_0_world_prepare().bits(0) });
            wcl.core_0_world_update()
                .write(|w| unsafe { w.core_0_update().bits(1) });
        }
    }
}
