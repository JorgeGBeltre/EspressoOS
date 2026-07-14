use esp_hal::xtensa_lx_rt::exception::{Context, ExceptionCause};

const SYSCALL_INSN_LEN: u32 = 3;

extern "C" {

    fn __user_exception(cause: ExceptionCause, save_frame: &mut Context);
}

#[no_mangle]
#[link_section = ".rwtext"]
unsafe extern "C" fn __exception(cause: ExceptionCause, save_frame: &mut Context) {
    if cause == ExceptionCause::Syscall {
        let num = save_frame.A2 as usize;
        let args = [
            save_frame.A3 as usize,
            save_frame.A4 as usize,
            save_frame.A5 as usize,
            save_frame.A6 as usize,
            save_frame.A7 as usize,
            save_frame.A8 as usize,
        ];
        let ret = crate::syscall::dispatch(num, &args, save_frame as *mut Context);

        if !crate::scheduler::take_restart_syscall() {
            save_frame.A2 = ret as u32;
            save_frame.PC = save_frame.PC.wrapping_add(SYSCALL_INSN_LEN);
        }

        if crate::scheduler::need_resched() {
            crate::scheduler::preempt_switch(save_frame);
        }

        let _ = crate::scheduler::process::check_signals(save_frame);
        return;
    }

    __user_exception(cause, save_frame);
}
