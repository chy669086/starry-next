use crate::process::signal::send_signal_to_proc;
use crate::syscall_body;
use crate::syscall_imp::{SigMaskFlag, SIGSET_SIZE_IN_BYTE};
use axtask::{current, TaskExtRef};

pub fn sys_sigprocmask(
    flag: usize,
    new_mask: *const usize,
    old_mask: *mut usize,
    sigsetsize: usize,
) -> isize {
    debug!(
        "sys_sigprocmask <= {}, {:p}, {:p}, {}",
        flag, new_mask, old_mask, sigsetsize
    );
    syscall_body!(sys_sigprocmask, {
        let flag = SigMaskFlag::from(flag);
        if sigsetsize != SIGSET_SIZE_IN_BYTE {
            return Err(axerrno::LinuxError::EINVAL);
        }

        let task = current();
        let proc = task.task_ext().get_proc().unwrap();

        let mut sig_modules = proc.signal_module.lock();
        let sig_module = sig_modules.get_mut(&task.id().as_u64()).unwrap();
        if old_mask as usize != 0 {
            unsafe {
                *old_mask = sig_module.sig_set.mask;
            }
        }

        if new_mask as usize != 0 {
            let now_mask = unsafe { *new_mask };
            match flag {
                SigMaskFlag::Block => {
                    sig_module.sig_set.mask |= now_mask;
                }
                SigMaskFlag::Unblock => {
                    sig_module.sig_set.mask &= !now_mask;
                }
                SigMaskFlag::Setmask => {
                    sig_module.sig_set.mask = now_mask;
                }
            }
        }

        Ok(0)
    })
}

pub(crate) fn sys_kill(pid: isize, signum: isize) -> isize {
    debug!("sys_kill <= {}, {}", pid, signum);
    syscall_body!(sys_kill, {
        if pid > 0 && signum > 0 {
            let _ = send_signal_to_proc(pid as u64, signum, None);
            Ok(0)
        } else if pid == 0 {
            Err(axerrno::LinuxError::ESRCH)
        } else {
            Err(axerrno::LinuxError::EINVAL)
        }
    })
}
