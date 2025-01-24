mod api;

use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use axerrno::AxResult;
use axmm::AddrSpace;
use axsync::Mutex;
use axtask::{current, yield_now, AxTaskRef, TaskExtRef, TaskInner};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use axhal::arch::UspaceContext;
use crate::flag::{CloneFlags, WaitStatus};
use crate::task::{read_trap_frame_from_kstack, TaskExt};

pub use api::*;
pub type AxProcessRef = Arc<Process>;

pub struct Thread(u64, AxTaskRef);

pub struct Process {
    /// 进程 ID
    pub pid: u64,
    /// 父进程 ID
    pub ppid: AtomicU64,
    /// 子进程
    pub children: Mutex<Vec<AxProcessRef>>,
    /// 线程
    pub threads: Mutex<Vec<Thread>>,
    /// 地址空间
    pub aspace: Arc<Mutex<AddrSpace>>,
    /// 退出码
    pub exit_code: AtomicI32,
    /// 堆底，用于 sbrk 系统调用
    pub heap_bottom: AtomicU64,
    /// 堆顶，用于 sbrk 系统调用
    pub heap_top: AtomicU64,
    /// 当前堆顶
    pub heap_current: AtomicU64,
    /// 线程编号
    pub tid_counter: AtomicU64,
    /// 进程状态
    pub is_exited: AtomicBool,
}

const BRK_BOTTOM: u64 = 0x40000000;
const BRK_TOP: u64 = 0x80000000;

impl Process {
    pub fn from_task(task: AxTaskRef, ppid: u64) -> Self {
        let process = Self::new(ppid, task.task_ext().aspace.clone());
        process.set_main_thread(task);
        process
    }
    pub fn new(ppid: u64, aspace: Arc<Mutex<AddrSpace>>) -> Self {
        static PID_COUNTER: AtomicU64 = AtomicU64::new(1);
        let pid = PID_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self {
            pid,
            ppid: AtomicU64::new(ppid),
            children: Mutex::new(Vec::new()),
            threads: Mutex::new(Vec::new()),
            aspace,
            exit_code: AtomicI32::new(0),
            heap_bottom: AtomicU64::new(BRK_BOTTOM),
            heap_top: AtomicU64::new(BRK_TOP),
            heap_current: AtomicU64::new(BRK_BOTTOM),
            tid_counter: AtomicU64::new(0),
            is_exited: AtomicBool::new(false),
        }
    }

    pub fn state(&self) -> axtask::TaskState {
        if self.is_exited.load(Ordering::Relaxed) {
            axtask::TaskState::Exited
        } else {
            axtask::TaskState::Running
        }
    }

    pub fn main_thread(&self) -> AxTaskRef {
        self.threads.lock()[0].1.clone()
    }

    pub fn clone_proc(
        &self,
        flags: usize,
        stack: Option<usize>,
        ptid: usize,
        tls: usize,
        ctid: usize,
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

        let new_task_ext = TaskExt::new(new_uctx, new_aspace);

        if clone_flags.contains(CloneFlags::CLONE_FS) {
            new_task_ext.init_fs_shared()
        }

        if clone_flags.contains(CloneFlags::CLONE_CHILD_CLEARTID) {
            new_task_ext.set_clear_child_tid(ctid as u64);
        }

        new_task_ext.init_ns();
        new_task.init_task_ext(new_task_ext);

        // 设置父子进程关系
        let new_task_ref = axtask::spawn_task(new_task);
        let proc = new_process(self.pid, new_task_ref.clone());
        let pid = proc.pid;
        self.children.lock().push(proc.clone());
        new_task_ref.task_ext().proc.init_once(Arc::downgrade(&proc));

        Ok(pid)
    }

    fn next_tid(&self) -> u64 {
        self.tid_counter.fetch_add(1, Ordering::Relaxed) as u64
    }

    pub fn set_main_thread(&self, thread: AxTaskRef) {
        let mut threads = self.threads.lock();
        assert!(threads.is_empty());
        threads.push(Thread(self.next_tid(), thread));
    }

    pub fn add_thread(&self, thread: AxTaskRef) {
        self.threads.lock().push(Thread(self.next_tid(), thread));
    }

    pub fn exit_thread(&self, thread: AxTaskRef, status: i32) {
        let mut threads = self.threads.lock();
        let index = threads
            .iter()
            .position(|t| t.1.id() == thread.id())
            .unwrap();
        // 主线程退出时，退出整个进程
        if index == 0 {
            drop(threads);
            self.exit(status);
            return;
        }

        let thread = threads.remove(index);
        threads[0].1.add_child_time(&thread.1);
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Relaxed)
    }

    pub fn exit(&self, code: i32) {
        for child in self.children.lock().iter_mut() {
            child.ppid.store(1, Ordering::SeqCst);
        }

        // 等待其他线程退出
        while self.threads.lock().len() > 1 {
            yield_now();
        }

        self.exit_code.store(code, Ordering::Relaxed);
        self.is_exited.store(true, Ordering::Relaxed);
        remove_process(self.pid);
    }
}
