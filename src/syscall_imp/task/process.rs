use crate::mm::load_elf_with_arg;
use crate::process::wait_pid;
use crate::syscall_body;
use crate::task::TaskExt;
use crate::{flag::WaitStatus, task::write_trap_frame_to_kstack};
use alloc::string::String;
use alloc::vec::Vec;
use arceos_posix_api::char_ptr_to_str;
use axhal::arch::UspaceContext;
use axtask::{current, TaskExtRef};
use core::ffi::c_char;

pub(crate) fn sys_clone(
    flags: usize,
    user_stack: usize,
    ptid: usize,
    tls: usize,
    child_tid: usize,
) -> isize {
    syscall_body!(sys_clone, {
        let stack = if user_stack == 0 {
            None
        } else {
            Some(user_stack)
        };

        let curr_task = current();

        if let Ok(new_task_id) = curr_task
            .task_ext()
            .get_proc()
            .unwrap()
            .clone_proc(flags, stack, ptid, tls, child_tid)
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

/// execve 系统调用
pub(crate) fn sys_execve(
    file_name: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> isize {
    let curr = current();
    let proc = curr.task_ext().get_proc().unwrap();
    if proc.threads.lock().len() > 1 {
        warn!("execve: now only support single-threaded process");
        return -1;
    }

    let Ok(path) = char_ptr_to_str(file_name) else {
        return -1;
    };

    // Copy the path, argv, and envp from user space to kernel space
    let path = String::from(path);
    let argv = unsafe { copy_from_ptr(argv) };
    let envp = unsafe { copy_from_ptr(envp) };

    let mut aspace = proc.aspace.lock();

    // Clear the address space
    aspace.clear();

    // Load the ELF file
    let Ok((entry_vaddr, ustack_top)) = load_elf_with_arg(&path, &mut aspace, &argv, &envp) else {
        return -1;
    };

    // 可能造成了 UB
    // TODO: 不使用裸指针
    let task_ext = unsafe { &mut *(curr.task_ext_ptr() as *mut TaskExt) };
    task_ext.uctx = UspaceContext::new(entry_vaddr.as_usize(), ustack_top, argv.len());

    // Write the trap frame to the kernel stack
    let trap_frame = task_ext.uctx.get_inner();
    write_trap_frame_to_kstack(curr.kernel_stack_top().unwrap().as_usize(), trap_frame);

    drop(aspace);

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

/// Safety: ptr is a valid pointer to a null-terminated array of pointers to null-terminated strings
unsafe fn copy_from_ptr(ptr: *const *const c_char) -> Vec<String> {
    let mut res = Vec::new();
    let mut i = 0;
    loop {
        let p = unsafe { *ptr.add(i) };
        if p.is_null() {
            break;
        }
        let Ok(s) = char_ptr_to_str(p) else {
            return Vec::new();
        };

        let mut str = String::from(s);
        str.push('\0');
        res.push(str);
        i += 1;
    }
    res
}
