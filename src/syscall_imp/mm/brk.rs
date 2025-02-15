use axhal::paging::MappingFlags;
use axtask::{current, TaskExtRef};
use core::sync::atomic::Ordering::SeqCst;
use memory_addr::{MemoryAddr, VirtAddr};

pub(crate) fn sys_brk(addr: *mut u8) -> isize {
    let curr = current();
    let curr_ext = curr.task_ext();
    let proc = curr_ext.get_proc().unwrap();
    let brk = proc.heap_current.load(SeqCst) as usize;
    let bottom = proc.heap_bottom.load(SeqCst) as usize;
    let addr = addr as usize;
    // 如果 addr 为 0，则返回当前 brk 地址
    if addr == 0 {
        return brk as isize;
    }
    // 如果 addr 小于 brk_bottom，则返回 -1
    if addr < bottom {
        return -1;
    }
    let mut aspace = proc.aspace.lock();

    // 如果 addr 小于 brk，则释放 addr 到 brk 之间的内存
    if addr < brk {
        let start_addr = VirtAddr::from(addr).align_up_4k();
        let end_addr = VirtAddr::from(brk).align_up_4k();
        if aspace
            .unmap(start_addr, end_addr.sub(start_addr.as_usize()).as_usize())
            .is_err()
        {
            return -1;
        }
    } else {
        let start_addr = VirtAddr::from(brk).align_up_4k();
        let end_addr = VirtAddr::from(addr).align_up_4k();
        let permission = MappingFlags::all();

        match proc.alloc_range_lazy(start_addr, end_addr, permission) {
            Ok(_) | Err(axerrno::AxError::InvalidInput) => {}
            Err(_) => return -1,
        }
    }

    proc.heap_current.store(addr as u64, SeqCst);
    addr as isize
}
