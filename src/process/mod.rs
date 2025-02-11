mod api;
pub mod signal;

use crate::flag::{CloneFlags, WaitStatus};
use crate::process::signal::SignalModule;
use crate::task::{read_trap_frame_from_kstack, TaskExt};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
pub use api::*;
use axerrno::AxResult;
use axhal::arch::UspaceContext;
use axhal::paging::MappingFlags;
use axmm::AddrSpace;
use axsync::Mutex;
use axtask::{current, yield_now, AxTaskRef, TaskExtRef, TaskInner};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use memory_addr::{MemoryAddr, VirtAddr};

pub type AxProcessRef = Arc<Process>;

pub struct Process {
    /// 进程 ID
    pub pid: u64,
    /// 父进程 ID
    pub ppid: AtomicU64,
    /// 子进程
    pub children: Mutex<Vec<AxProcessRef>>,
    /// 线程，tid -> thread
    pub threads: Mutex<BTreeMap<u64, AxTaskRef>>,
    pub taskid2tid: Mutex<BTreeMap<u64, u64>>,
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
    /// 信号处理
    pub signal_module: Mutex<BTreeMap<u64, SignalModule>>,
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
        static PID_COUNTER: AtomicU64 = AtomicU64::new(2);
        let pid = PID_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self {
            pid,
            ppid: AtomicU64::new(ppid),
            children: Mutex::new(Vec::new()),
            threads: Mutex::new(BTreeMap::new()),
            aspace,
            exit_code: AtomicI32::new(0),
            heap_bottom: AtomicU64::new(BRK_BOTTOM),
            heap_top: AtomicU64::new(BRK_TOP),
            heap_current: AtomicU64::new(BRK_BOTTOM),
            tid_counter: AtomicU64::new(1),
            is_exited: AtomicBool::new(false),
            signal_module: Mutex::new(BTreeMap::new()),
            taskid2tid: Mutex::new(BTreeMap::new()),
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
        self.threads.lock()[&1].clone()
    }

    fn next_tid(&self) -> u64 {
        self.tid_counter.fetch_add(1, Ordering::Relaxed)
    }

    pub fn set_main_thread(&self, thread: AxTaskRef) {
        assert_eq!(self.tid_counter.load(Ordering::Relaxed), 1);
        self.add_thread(thread);
    }

    pub fn get_tid_from_taskid(&self, taskid: u64) -> Option<u64> {
        self.taskid2tid.lock().get(&taskid).cloned()
    }

    pub fn get_tid_from_task(&self, task: &AxTaskRef) -> Option<u64> {
        self.get_tid_from_taskid(task.id().as_u64())
    }

    pub fn add_thread(&self, thread: AxTaskRef) {
        let tid = self.next_tid();
        self.signal_module
            .lock()
            .insert(tid, SignalModule::new(None));
        self.taskid2tid.lock().insert(thread.id().as_u64(), tid);
        self.threads.lock().insert(tid, thread);
    }

    pub fn exit_thread(&self, thread: AxTaskRef, status: i32) {
        let tid = self
            .taskid2tid
            .lock()
            .remove(&thread.id().as_u64())
            .unwrap();
        self.signal_module.lock().remove(&tid);
        // 主线程退出时，退出整个进程
        if tid == 1 {
            self.exit(status);
            return;
        }
        let mut threads = self.threads.lock();
        let _thread = threads.remove(&tid).unwrap();
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Relaxed)
    }

    pub fn exit(&self, code: i32) {
        for child in self.children.lock().iter_mut() {
            child.ppid.store(1, Ordering::SeqCst);
        }
        self.is_exited.store(true, Ordering::Relaxed);

        // 等待其他线程退出
        // TODO: 直接退出其他线程
        while self.threads.lock().len() > 1 {
            yield_now();
        }

        self.exit_code.store(code, Ordering::Relaxed);
        remove_process(self.pid);
        debug!("Process {} exited with code {}", self.pid, code);
    }

    pub fn alloc_range_lazy(&self, start: VirtAddr, end: VirtAddr) -> AxResult<()> {
        if start > end {
            return Err(axerrno::AxError::InvalidInput);
        }
        let start = start.align_down_4k();
        let end = end.align_up_4k();
        let mut aspace = self.aspace.lock();
        aspace.map_alloc(start, end - start, MappingFlags::all(), false)?;
        Ok(())
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

        // 共享文件描述符
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
        new_task_ref
            .task_ext()
            .proc
            .init_once(Arc::downgrade(&proc));

        Ok(pid)
    }
}
