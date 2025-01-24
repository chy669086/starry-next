use crate::flag::WaitStatus;
use crate::mm::load_elf;
use crate::process::wait_pid;
use crate::syscall_body;
use crate::task::TaskExt;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use arceos_posix_api::char_ptr_to_str;
use axerrno::AxResult;
use axhal::arch::UspaceContext;
use axtask::{current, TaskExtRef};
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
    let argv = unsafe { copy_from_ptr(argv) };
    let envp = unsafe { copy_from_ptr(envp) };

    aspace.clear();

    let (entry_vaddr, ustack_top) = load_elf(&path, &mut aspace).unwrap();

    let task_ext = unsafe { &mut *(curr.task_ext_ptr() as *mut TaskExt) };
    task_ext.uctx = UspaceContext::new(entry_vaddr.as_usize(), ustack_top, argv.len());

    let argv_ptr = alloc_user_argv(argv).unwrap();
    let envp_ptr = alloc_user_argv(envp).unwrap();

    task_ext.uctx.set_arg(argv_ptr, envp_ptr);

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

fn alloc_user_argv(argv: Vec<String>) -> AxResult<usize> {
    let argv = Box::leak(argv.into_boxed_slice());

    let argv_ptr = argv
        .iter()
        .map(|s| s.as_ptr() as usize)
        .chain(vec![0].into_iter())
        .collect::<Vec<_>>();

    let argv_ptr = argv_ptr.into_boxed_slice();

    let ptr = Box::leak(argv_ptr);

    Ok(ptr.as_ptr() as usize)
}

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
