use crate::flag::WaitStatus;
use crate::mm::{load_elf, load_user_app};
use crate::task::{wait_pid, TaskExt};
use crate::{mm, syscall_body, task};
use alloc::string::String;
use alloc::sync::Arc;
use arceos_posix_api::char_ptr_to_str;
use axhal::arch::UspaceContext;
use axmm::kernel_page_table_root;
use axsync::Mutex;
use axtask::{current, yield_now, TaskExtMut, TaskExtRef};
use core::ffi::c_char;

pub(crate) fn sys_clone(
    flags: usize,
    user_stack: usize,
    ptid: usize,
    arg3: usize,
    arg4: usize,
) -> isize {
    syscall_body!(sys_clone, {
        let tls = arg3;
        let child_tid = arg4;

        let stack = if user_stack == 0 {
            None
        } else {
            Some(user_stack)
        };

        let curr_task = current();

        if let Ok(new_task_id) = curr_task
            .task_ext()
            .clone_task(flags, stack, ptid, tls, child_tid)
        {
            Ok(new_task_id as isize)
        } else {
            Err(axerrno::LinuxError::ENOMEM)
        }
    })
}

pub(crate) fn sys_wait4(pid: i32, exit_code_ptr: *mut i32, _option: u32) -> usize {
    syscall_body!(sys_wait4, {
        loop {
            match wait_pid(pid, exit_code_ptr, _option) {
                Ok(child_pid) => return Ok(child_pid as usize),
                Err(WaitStatus::NotExist) => return Err(axerrno::LinuxError::ECHILD),
                Err(WaitStatus::Running) => {
                    axtask::yield_now();
                }
                _ => panic!("Unexpected wait status"),
            }
        }
    })
}

pub(crate) fn sys_execve(
    file_name: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> isize {
    let curr = current();
    let mut aspace = curr.task_ext().aspace.lock();
    if Arc::strong_count(&curr.task_ext().aspace) != 1 {
        warn!("execve: aspace is shared, not supported yet!");
        return -1;
    }
    let Ok(path) = char_ptr_to_str(file_name) else {
        return -1;
    };

    let path = String::from(path);

    aspace.clear();

    let (entry_vaddr, ustack_top) = load_elf(&path, &mut aspace).unwrap();

    drop(aspace);

    let task_ext = unsafe { &mut *(curr.task_ext_ptr() as *mut TaskExt) };
    task_ext.uctx = UspaceContext::new(entry_vaddr.as_usize(), ustack_top, 0);

    let kstack_top = curr.kernel_stack_top().unwrap();
    info!(
        "Enter user space: entry={:#x}, ustack={:#x}, kstack={:#x}",
        task_ext.uctx.get_ip(),
        task_ext.uctx.get_sp(),
        kstack_top,
    );

    #[cfg(feature = "tls")]
    {
        task_ctx.tp = axhal::arch::read_thread_pointer();
        unsafe { axhal::arch::write_thread_pointer(next_ctx.tp) };
    }

    unsafe {
        task_ext.uctx.enter_uspace(kstack_top);
    }
}
