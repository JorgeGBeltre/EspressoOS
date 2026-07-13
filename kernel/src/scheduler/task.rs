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

        let stack_top = unsafe { base.add(size) };

        let context = context::init_task_stack(stack_top, super::task_trampoline, tid as usize);

        Ok(Box::new(Task {
            tid,
            name: String::from(name),
            state: TaskState::Ready,
            priority,
            context,
            stack_base: base,
            stack_size: size,
            exit_code: 0,
            is_user: false,
            affinity: None,
            start_entry: entry,
            start_arg: arg,
        }))
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
