use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use crate::{config, loader};
use axerrno::AxResult;
use axhal::{
    paging::MappingFlags,
    trap::{register_trap_handler, PAGE_FAULT},
};
use axmm::AddrSpace;
use axtask::TaskExtRef;
use memory_addr::VirtAddr;

/// Load a user app.
///
/// # Returns
/// - The first return value is the entry point of the user app.
/// - The second return value is the top of the user stack.
/// - The third return value is the address space of the user app.
pub fn load_user_app(app_name: &str) -> AxResult<(VirtAddr, VirtAddr, AddrSpace)> {
    let mut uspace = axmm::new_user_aspace(
        VirtAddr::from_usize(config::USER_SPACE_BASE),
        config::USER_SPACE_SIZE,
    )?;

    let (entry, ustack_pointer) = load_elf(app_name, &mut uspace)?;

    Ok((entry, ustack_pointer, uspace))
}

pub fn load_elf_with_arg(
    app_name: &str,
    uspace: &mut AddrSpace,
    argv: &[String],
    envp: &[String],
) -> AxResult<(VirtAddr, VirtAddr)> {
    let elf_info = loader::load_elf(app_name, uspace.base());
    for segement in elf_info.segments {
        debug!(
            "Mapping ELF segment: [{:#x?}, {:#x?}) flags: {:#x?}",
            segement.start_vaddr,
            segement.start_vaddr + segement.size,
            segement.flags
        );
        uspace.map_alloc(segement.start_vaddr, segement.size, segement.flags, true)?;

        if segement.data.is_empty() {
            continue;
        }

        uspace.write(segement.start_vaddr + segement.offset, segement.data)?;

        // TDOO: flush the I-cache
    }

    // The user stack is divided into two parts:
    // `ustack_start` -> `ustack_pointer`: It is the stack space that users actually read and write.
    // `ustack_pointer` -> `ustack_end`: It is the space that contains the arguments, environment variables and auxv passed to the app.
    //  When the app starts running, the stack pointer points to `ustack_pointer`.
    let ustack_end = VirtAddr::from_usize(config::USER_STACK_TOP);
    let ustack_size = config::USER_STACK_SIZE;
    let ustack_start = ustack_end - ustack_size;
    debug!(
        "Mapping user stack: {:#x?} -> {:#x?}",
        ustack_start, ustack_end
    );
    // FIXME: Add more arguments and environment variables
    let (stack_data, ustack_pointer) = kernel_elf_parser::get_app_stack_region(
        argv,
        envp,
        &elf_info.auxv,
        ustack_start,
        ustack_size,
    );
    uspace.map_alloc(
        ustack_start,
        ustack_size,
        MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
        true,
    )?;

    uspace.write(VirtAddr::from_usize(ustack_pointer), stack_data.as_slice())?;

    Ok((elf_info.entry, VirtAddr::from_usize(ustack_pointer)))
}

pub fn load_elf(app_name: &str, uspace: &mut AddrSpace) -> AxResult<(VirtAddr, VirtAddr)> {
    load_elf_with_arg(app_name, uspace, &[app_name.to_string()], &[])
}

#[register_trap_handler(PAGE_FAULT)]
fn handle_page_fault(vaddr: VirtAddr, access_flags: MappingFlags, is_user: bool) -> bool {
    if !is_user {
        warn!(
            "Kernel page fault at {:#x}, access_flags: {:#x?}",
            vaddr, access_flags
        );
    }
    let task = axtask::current();
    if unsafe { task.task_ext_ptr().is_null() } {
        error!("No task extended data found for the current task");
        return false;
    }
    if !task
        .task_ext()
        .get_proc()
        .unwrap()
        .aspace
        .lock()
        .handle_page_fault(vaddr, access_flags)
    {
        warn!(
            "{}: segmentation fault at {:#x}, exit!",
            axtask::current().id_name(),
            vaddr
        );
        crate::syscall_imp::sys_exit(-1);
    }
    true
}
