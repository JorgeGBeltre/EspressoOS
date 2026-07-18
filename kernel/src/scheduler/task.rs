#![allow(dead_code)]

use core::alloc::Layout;

use crate::arch::xtensa::context::{self, Context};
use crate::prelude::*;

pub type Tid = u32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,
    Zombie,
}

const STACK_ALIGN: usize = 16;

/// Patrón de pintado de pila para el watermark (D-10). La pila sin usar conserva este
/// valor tras `Task::new`; `stack_high_water` lo escanea para medir el uso máximo.
const STACK_PAINT: u32 = 0xDEAD_BEEF;

const fn align_up(n: usize, align: usize) -> usize {
    n.saturating_add(align - 1) & !(align - 1)
}

pub struct Task {
    pub tid: Tid,
    pub name: String,
    pub state: TaskState,
    pub priority: u8,
    pub context: Context,
    pub stack_base: *mut u8,
    pub stack_size: usize,
    pub exit_code: i32,
    pub is_user: bool,
    pub affinity: Option<usize>,

    pub(super) start_entry: fn(usize),

    pub(super) start_arg: usize,
}

unsafe impl Send for Task {}

impl Task {
    pub(super) fn new(
        tid: Tid,
        name: &str,
        entry: fn(usize),
        arg: usize,
        stack_size: usize,
        priority: u8,
        is_user: bool,
    ) -> KResult<Box<Task>> {
        let requested = if stack_size == 0 {
            layout::DEFAULT_STACK_SIZE
        } else {
            stack_size
        };
        let size = align_up(requested, STACK_ALIGN);
        let alloc_layout =
            Layout::from_size_align(size, STACK_ALIGN).map_err(|_| KError::InvalidArgument)?;

        let base = unsafe { alloc::alloc::alloc(alloc_layout) };
        if base.is_null() {
            return Err(KError::NoMem);
        }

        // D-10 (watermark): pinta toda la pila con STACK_PAINT ANTES de escribir el frame
        // inicial. La pila crece hacia abajo desde stack_top; la zona baja (en la base)
        // que el task nunca toca conserva el patrón, y stack_high_water() lo escanea para
        // medir el uso. init_task_stack (justo debajo) sobrescribe el tope con el frame.
        unsafe {
            core::slice::from_raw_parts_mut(base as *mut u32, size / 4).fill(STACK_PAINT);
        }

        let stack_top = unsafe { base.add(size) };

        let context =
            context::init_task_stack(stack_top, super::task_trampoline, tid as usize, is_user);

        Ok(Box::new(Task {
            tid,
            name: String::from(name),
            state: TaskState::Ready,
            priority,
            context,
            stack_base: base,
            stack_size: size,
            exit_code: 0,
            is_user,
            affinity: None,
            start_entry: entry,
            start_arg: arg,
        }))
    }

    /// Uso máximo (high-water) de esta pila en bytes, midiendo cuánto del patrón
    /// STACK_PAINT (pintado en `new`) sigue intacto desde `stack_base` hacia arriba. La
    /// pila crece hacia abajo, así que la zona intacta está en la base. Caveat: si el
    /// task escribió el patrón como dato, subestima — suficiente para un margen (D-10).
    pub fn stack_high_water(&self) -> usize {
        if self.stack_base.is_null() || self.stack_size == 0 {
            return 0;
        }
        let words = self.stack_size / 4;
        let p = self.stack_base as *const u32;
        let mut untouched = 0usize;
        unsafe {
            while untouched < words && p.add(untouched).read_volatile() == STACK_PAINT {
                untouched += 1;
            }
        }
        self.stack_size - untouched * 4
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if !self.stack_base.is_null() && self.stack_size > 0 {
            if let Ok(dealloc_layout) = Layout::from_size_align(self.stack_size, STACK_ALIGN) {
                unsafe { alloc::alloc::dealloc(self.stack_base, dealloc_layout) };
            }
        }
    }
}
