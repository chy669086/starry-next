use crate::flag::{CloneFlags, WaitStatus};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use arceos_posix_api::FD_TABLE;
use axerrno::AxResult;
use axfs::{CURRENT_DIR, CURRENT_DIR_PATH};
use axhal::arch::{TrapFrame, UspaceContext};
use axmm::{kernel_page_table_root, AddrSpace};
use axns::{AxNamespace, AxNamespaceIf, AxResource};
use axsync::Mutex;
use axtask::{current, AxTaskRef, TaskExtRef, TaskInner};
use core::ops::Sub;
use core::sync::atomic::AtomicU64;

/// Task extended data for the monolithic kernel.
pub struct TaskExt {
    /// The parent process ID.
    pub ppid: usize,
    /// The process ID.
    pub proc_id: usize,
    /// children process
    pub children: Mutex<Vec<AxTaskRef>>,
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
    pub fn new(proc_id: usize, uctx: UspaceContext, aspace: Arc<Mutex<AddrSpace>>) -> Self {
        Self {
            proc_id,
            ppid: 1,
            children: Mutex::new(Vec::new()),
            uctx,
            clear_child_tid: AtomicU64::new(0),
            aspace,
            ns: AxNamespace::new_thread_local(),
        }
    }

    pub fn clone_task(
        &self,
        flags: usize,
        stack: Option<usize>,
        _ptid: usize,
        _tls: usize,
        _ctid: usize,
    ) -> AxResult<u64> {
        let clone_flags = CloneFlags::from_bits((flags & !0x3f) as u32).unwrap();

        let mut new_task = TaskInner::new(
            || {
                let curr = current();
                let kstack_top = curr.kernel_stack_top().unwrap();
                info!(
                    "Enter user space: entry={:#x}, ustack={:#x}, kstack={:#x}",
                    curr.task_ext().uctx.get_ip(),
                    curr.task_ext().uctx.get_sp(),
                    kstack_top,
                );
                unsafe { curr.task_ext().uctx.enter_uspace(kstack_top) };
            },
            String::from(current().id_name()),
            crate::config::KERNEL_STACK_SIZE,
        );

        let curr = current();
        let mut trap_frame =
            read_trap_frame_from_kstack(curr.kernel_stack_top().unwrap().as_usize());

        let new_aspace = if clone_flags.contains(CloneFlags::CLONE_VM) {
            self.aspace.clone()
        } else {
            let new_aspace = AddrSpace::from_exited_space(&self.aspace.lock())?;
            Arc::new(Mutex::new(new_aspace))
        };

        new_task
            .ctx_mut()
            .set_page_table_root(new_aspace.lock().page_table_root());

        trap_frame.regs.a0 = 0;
        trap_frame.sepc += 4;

        if let Some(stack) = stack {
            trap_frame.regs.sp = stack;
        }

        let new_uctx = UspaceContext::from(&trap_frame);

        let task_id = new_task.id().as_u64();
        let new_task_ext = TaskExt::new(task_id as usize, new_uctx, new_aspace);

        new_task_ext.init_ns();
        new_task.init_task_ext(new_task_ext);

        let new_task_ref = axtask::spawn_task(new_task);

        self.children.lock().push(new_task_ref);

        Ok(task_id)
    }

    pub(crate) fn clear_child_tid(&self) -> u64 {
        self.clear_child_tid
            .load(core::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn set_clear_child_tid(&self, clear_child_tid: u64) {
        self.clear_child_tid
            .store(clear_child_tid, core::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn init_ns(&self) {
        unsafe {
            FD_TABLE.init_new_from(&self.ns);
            CURRENT_DIR.init_new_from(&self.ns);
            CURRENT_DIR_PATH.init_new_from(&self.ns);
        }
        FD_TABLE
            .deref_from(&self.ns)
            .init_new(FD_TABLE.copy_inner());
        CURRENT_DIR
            .deref_from(&self.ns)
            .init_new(CURRENT_DIR.copy_inner());
        CURRENT_DIR_PATH
            .deref_from(&self.ns)
            .init_new(CURRENT_DIR_PATH.copy_inner());
    }
}

impl Drop for TaskExt {
    fn drop(&mut self) {
        // TODO: 将所有子进程的父进程设置为1
        unsafe {
            FD_TABLE.deref_from(&self.ns);
            CURRENT_DIR.deref_from(&self.ns);
            CURRENT_DIR_PATH.deref_from(&self.ns);
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
    task.init_task_ext(TaskExt::new(task.id().as_u64() as usize, uctx, aspace));
    task.task_ext().init_ns();

    axtask::spawn_task(task)
}

pub(crate) fn wait_pid(pid: i32, exit_code_ptr: *mut i32, _option: u32) -> Result<u64, WaitStatus> {
    if pid <= 0 {
        return wait_pid_nagative(pid, exit_code_ptr, _option);
    }

    let curr_task = current();
    let mut proc_status = WaitStatus::NotExist;

    let child = curr_task
        .task_ext()
        .children
        .lock()
        .iter()
        .enumerate()
        .find(|(_id, child)| child.id().as_u64() as i32 == pid)
        .map(|(id, child)| (id, child.clone()));

    let Some((loc, child)) = child else {
        return Err(WaitStatus::NotExist);
    };

    let state = child.state();
    if state == axtask::TaskState::Running {
        proc_status = WaitStatus::Running;
    } else if state == axtask::TaskState::Exited {
        let exit_code = child.exit_code();
        if !exit_code_ptr.is_null() {
            unsafe {
                *exit_code_ptr = exit_code;
            }
        }

        curr_task.task_ext().children.lock().remove(loc);

        return Ok(child.id().as_u64());
    }

    Err(proc_status)
}

fn wait_pid_nagative(pid: i32, exit_code_ptr: *mut i32, _option: u32) -> Result<u64, WaitStatus> {
    assert!(pid <= 0);

    if pid == 0 {
        warn!("wait process group is not implemented");
    }

    let curr_task = current();
    let mut proc_status = WaitStatus::NotExist;
    let mut child_id = 0;

    for (id, task) in curr_task.task_ext().children.lock().iter().enumerate() {
        proc_status = WaitStatus::Running;
        if task.state() == axtask::TaskState::Exited {
            proc_status = WaitStatus::Exited;
            let exit_code = task.exit_code();
            if !exit_code_ptr.is_null() {
                unsafe {
                    *exit_code_ptr = exit_code;
                }
            }

            child_id = id;
            break;
        }
    }

    if proc_status == WaitStatus::Exited {
        let child = curr_task.task_ext().children.lock().remove(child_id);
        return Ok(child.id().as_u64());
    }

    Err(proc_status)
}

pub fn read_trap_frame_from_kstack(kstack_top: usize) -> TrapFrame {
    let trap_frame_size = core::mem::size_of::<TrapFrame>();
    let trap_frame_addr = kstack_top - trap_frame_size;
    unsafe { *(trap_frame_addr as *const TrapFrame) }
}
