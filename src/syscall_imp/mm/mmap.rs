use crate::syscall_body;
use crate::syscall_imp::fs::sys_read;
use axerrno::LinuxError;
use axhal::arch::read_page_table_root;
use axhal::paging::MappingFlags;
use axtask::{current, TaskExtRef};
use memory_addr::{MemoryAddr, VirtAddr, VirtAddrRange};

bitflags::bitflags! {
    /// permissions for sys_mmap
    ///
    /// See <https://github.com/bminor/glibc/blob/master/bits/mman.h>
    #[derive(Debug)]
    struct MmapProt: i32 {
        /// Page can be read.
        const PROT_READ = 1 << 0;
        /// Page can be written.
        const PROT_WRITE = 1 << 1;
        /// Page can be executed.
        const PROT_EXEC = 1 << 2;
    }
}

impl From<MmapProt> for MappingFlags {
    fn from(value: MmapProt) -> Self {
        let mut flags = MappingFlags::USER;
        if value.contains(MmapProt::PROT_READ) {
            flags |= MappingFlags::READ;
        }
        if value.contains(MmapProt::PROT_WRITE) {
            flags |= MappingFlags::WRITE;
        }
        if value.contains(MmapProt::PROT_EXEC) {
            flags |= MappingFlags::EXECUTE;
        }
        flags
    }
}

bitflags::bitflags! {
    /// flags for sys_mmap
    ///
    /// See <https://github.com/bminor/glibc/blob/master/bits/mman.h>
    #[derive(Debug)]
    struct MmapFlags: i32 {
        /// Share changes
        const MAP_SHARED = 1 << 0;
        /// Changes private; copy pages on write.
        const MAP_PRIVATE = 1 << 1;
        /// Map address must be exactly as requested, no matter whether it is available.
        const MAP_FIXED = 1 << 4;
        /// Don't use a file.
        const MAP_ANONYMOUS = 1 << 5;
        /// Don't check for reservations.
        const MAP_NORESERVE = 1 << 14;
        /// Allocation is for a stack.
        const MAP_STACK = 0x20000;
    }
}

pub(crate) fn sys_mmap(
    addr: *mut usize,
    length: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    offset: isize,
) -> usize {
    debug!(
        "sys_mmap <= {:#x} {} {:#x} {} {} {}",
        addr as usize, length, prot, flags, fd, offset
    );
    syscall_body!(sys_mmap, {
        let curr = current();
        let curr_ext = curr.task_ext();
        let mut aspace = curr_ext.aspace.lock();
        let permission_flags = MmapProt::from_bits_truncate(prot);
        // TODO: check illegal flags for mmap
        // An example is the flags contained none of MAP_PRIVATE, MAP_SHARED, or MAP_SHARED_VALIDATE.
        let map_flags = MmapFlags::from_bits_truncate(flags);

        let start_addr = if map_flags.contains(MmapFlags::MAP_FIXED) {
            VirtAddr::from(addr as usize)
        } else {
            aspace
                .find_free_area(
                    VirtAddr::from(addr as usize),
                    length,
                    VirtAddrRange::new(aspace.base(), aspace.end()),
                )
                .or(aspace.find_free_area(
                    aspace.base(),
                    length,
                    VirtAddrRange::new(aspace.base(), aspace.end()),
                ))
                .ok_or(LinuxError::ENOMEM)?
        };

        let populate = if fd == -1 {
            false
        } else {
            !map_flags.contains(MmapFlags::MAP_ANONYMOUS)
        };

        let end_addr = (start_addr + length).align_up_4k();

        aspace.map_alloc(
            start_addr.align_down_4k(),
            end_addr
                .sub(start_addr.align_down_4k().as_usize())
                .as_usize(),
            permission_flags.into(),
            true,
        )?;

        drop(aspace);

        if populate {
            let file_inner = arceos_posix_api::read_file(fd, offset as usize, length)?;

            let ptr = start_addr.as_mut_ptr();

            unsafe {
                core::ptr::copy_nonoverlapping(file_inner.as_ptr(), ptr, length);
            }
        }

        Ok(start_addr.as_usize())
    })
}

pub(crate) fn sys_munmap(addr: *mut usize, mut length: usize) -> i32 {
    syscall_body!(sys_munmap, {
        let curr = current();
        let curr_ext = curr.task_ext();
        let mut aspace = curr_ext.aspace.lock();
        length = memory_addr::align_up_4k(length);
        let start_addr = VirtAddr::from(addr as usize);
        aspace.unmap(start_addr, length)?;
        axhal::arch::flush_tlb(None);
        Ok(0)
    })
}
