use crate::syscall_body;
use arceos_posix_api as api;

pub(crate) fn sys_pipe2(fds: *mut i32, flags: i32) -> i32 {
    debug!("pipe2(fds: {:?}, flags: {:#x})", fds, flags);
    syscall_body!(sys_pipe2, {
        if flags != 0 {
            warn!("Now only support no flags for pipe2");
        }

        let mut fds = unsafe { core::slice::from_raw_parts_mut(fds, 2) };
        Ok(api::sys_pipe(&mut fds))
    })
}
