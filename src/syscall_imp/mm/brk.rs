use arceos_posix_api as api;
use axtask::{current, TaskExtRef};

pub(crate) fn sys_brk(addr: *mut u8) -> usize {
    let curr = current();
    let aspace = curr.task_ext().aspace.lock();

    // todo!()
    0
}
