use crate::flag::{CloneFlags, WaitStatus};
use crate::process::{new_process, AxProcessRef, Process};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use arceos_posix_api::FD_TABLE;
use axerrno::AxResult;
use axfs::{CURRENT_DIR, CURRENT_DIR_PATH};
use axhal::arch::{TrapFrame, UspaceContext};
use axmm::AddrSpace;
use axns::{AxNamespace, AxNamespaceIf};
use axsync::Mutex;
use axtask::{current, AxTaskRef, TaskExtRef, TaskInner};
use core::cell::UnsafeCell;
use core::sync::atomic::AtomicU64;

/// Task extended data for the monolithic kernel.
pub struct TaskExt {
    /// 所属进程
    pub proc: lazyinit::LazyInit<Weak<Process>>,
    /// The clear thread tid field
    ///
    /// See <https://manpages.debian.org/unstable/manpages-dev/set_tid_address.2.en.html#clear_child_tid>
    ///
    /// When the thread exits, the kernel clears the word at this address if it is not NULL.
    clear_child_tid: AtomicU64,
    /// The user space context.
    pub uctx: UspaceContext,
    /// The virtual memory address space.
    pub aspace: Arc<Mutex<AddrSpace>>,
    /// The resource namespace.
    pub ns: AxNamespace,
}

impl TaskExt {
    pub fn new(uctx: UspaceContext, aspace: Arc<Mutex<AddrSpace>>) -> Self {
        let ext = Self {
            proc: lazyinit::LazyInit::new(),
            uctx,
            clear_child_tid: AtomicU64::new(0),
            aspace,
            ns: AxNamespace::new_thread_local(),
        };
        ext.init_ns_space();
        ext
    }

    pub fn get_proc(&self) -> Option<AxProcessRef> {
        self.proc.get().and_then(|p| p.upgrade())
    }

    /// This function is used to initialize the namespace space.
    /// It is called when the task is created.
    fn init_ns_space(&self) {
        unsafe {
            FD_TABLE.init_new_from(&self.ns);
            CURRENT_DIR.init_new_from(&self.ns);
            CURRENT_DIR_PATH.init_new_from(&self.ns);
        }
    }

    pub(crate) fn clear_child_tid(&self) -> u64 {
        self.clear_child_tid
            .load(core::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn set_clear_child_tid(&self, clear_child_tid: u64) {
        self.clear_child_tid
            .store(clear_child_tid, core::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn init_fs_shared(&self) {
        FD_TABLE.deref_from(&self.ns).init_shared(FD_TABLE.share());
    }

    pub(crate) fn init_ns(&self) {
        FD_TABLE
            .deref_from(&self.ns)
            .init_or_ignore(FD_TABLE.copy_inner());
        CURRENT_DIR
            .deref_from(&self.ns)
            .init_or_ignore(CURRENT_DIR.copy_inner());
        CURRENT_DIR_PATH
            .deref_from(&self.ns)
            .init_or_ignore(CURRENT_DIR_PATH.copy_inner());
    }
}

impl Drop for TaskExt {
    fn drop(&mut self) {
        unsafe {
            FD_TABLE.drop_from(&self.ns);
            CURRENT_DIR.drop_from(&self.ns);
            CURRENT_DIR_PATH.drop_from(&self.ns);
        }
    }
}

struct AxNamespaceImpl;

#[crate_interface::impl_interface]
impl AxNamespaceIf for AxNamespaceImpl {
    #[inline(never)]
    fn current_namespace_base() -> *mut u8 {
        let current = axtask::current();
        // Safety: We only check whether the task extended data is null and do not access it.
        if unsafe { current.task_ext_ptr() }.is_null() {
            return axns::AxNamespace::global().base();
        }
        current.task_ext().ns.base()
    }
}

axtask::def_task_ext!(TaskExt);

pub fn spawn_user_task(aspace: Arc<Mutex<AddrSpace>>, uctx: UspaceContext) -> AxTaskRef {
    let mut task = TaskInner::new(
        || {
            let curr = axtask::current();
            let kstack_top = curr.kernel_stack_top().unwrap();
            info!(
                "Enter user space: entry={:#x}, ustack={:#x}, kstack={:#x}",
                curr.task_ext().uctx.get_ip(),
                curr.task_ext().uctx.get_sp(),
                kstack_top,
            );
            unsafe { curr.task_ext().uctx.enter_uspace(kstack_top) };
        },
        "userboot".into(),
        crate::config::KERNEL_STACK_SIZE,
    );
    task.ctx_mut()
        .set_page_table_root(aspace.lock().page_table_root());
    task.init_task_ext(TaskExt::new(uctx, aspace));
    task.task_ext().init_ns();

    let task = axtask::spawn_task(task);
    let proc = new_process(1, task.clone());
    task.task_ext().proc.init_once(Arc::downgrade(&proc));

    task
}

pub fn read_trap_frame_from_kstack(kstack_top: usize) -> TrapFrame {
    let trap_frame_size = core::mem::size_of::<TrapFrame>();
    let trap_frame_addr = kstack_top - trap_frame_size;
    unsafe { *(trap_frame_addr as *const TrapFrame) }
}
