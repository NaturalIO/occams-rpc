use std::{
    collections::vec_deque::VecDeque,
    future::Future,
    mem::swap,
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    task::*,
};

use super::client_task::*;
use crate::error::*;
use crossfire::{SendError, channel::LockedWaker, mpsc};
use rustc_hash::FxHashMap;
use sync_utils::waitgroup::WaitGroupGuard;

macro_rules! retry_with_err {
    ($sender:expr, $t:expr, $err:expr) => {
        if let Some(sender) = $sender.as_ref() {
            let retry_task = RetryTaskInfo { task: $t, task_err: $err.clone() };
            if let Err(SendError(rt)) = sender.send(Some(retry_task)) {
                // TODO FIXME
                rt.unwrap().task.set_result(Err($err));
            }
        }
    };
}

pub struct RpcClientTaskItem<T: RpcClientTask> {
    pub task: Option<T>,
    _upstream: Option<WaitGroupGuard>,
}

pub struct DelayTasksBatch<T: RpcClientTask> {
    tasks: FxHashMap<u64, RpcClientTaskItem<T>>,
}

pub struct RpcClientTaskNotifier<T: RpcClientTask> {
    server_id: u64,
    client_id: u64,
    pending_tasks_recv: mpsc::RxFuture<RpcClientTaskItem<T>, mpsc::SharedFutureBoth>,
    pending_tasks_sender: mpsc::TxFuture<RpcClientTaskItem<T>, mpsc::SharedFutureBoth>,
    pending_task_count: AtomicU64,

    sent_tasks: FxHashMap<u64, RpcClientTaskItem<T>>, // sent_tasks of the current second
    delay_tasks_queue: VecDeque<DelayTasksBatch<T>>,  // sent_tasks of past seconds
    sent_task_waker: Option<LockedWaker>,

    min_delay_seq: u64,
    task_timeout: usize, // in seconds
    // TODO what if seq reach max u64, should exit client
    processed_seq: u64,
    reg_stopped_flag: AtomicBool,
}

impl<T: RpcClientTask> RpcClientTaskNotifier<T> {
    pub fn new(server_id: u64, client_id: u64, task_timeout: usize, mut thresholds: usize) -> Self {
        if thresholds == 0 {
            thresholds = 500;
        }
        let (pending_tx, pending_rx) = mpsc::bounded_future_both(thresholds * 2);
        Self {
            server_id,
            client_id,
            pending_tasks_recv: pending_rx,
            pending_tasks_sender: pending_tx,
            pending_task_count: AtomicU64::new(0),
            sent_tasks: FxHashMap::default(),
            sent_task_waker: None,
            min_delay_seq: 0,
            task_timeout,
            delay_tasks_queue: VecDeque::with_capacity(task_timeout),
            processed_seq: 0,
            reg_stopped_flag: AtomicBool::new(false),
        }
    }

    pub fn pending_task_count_ref(&self) -> &AtomicU64 {
        &self.pending_task_count
    }

    pub fn clean_pending_tasks(
        &mut self, retry_tx: Option<&mpsc::TxUnbounded<Option<RetryTaskInfo<T>>>>,
    ) {
        loop {
            match self.pending_tasks_recv.try_recv() {
                Ok(task) => {
                    self.got_pending_task(task);
                }
                Err(_) => {
                    break;
                }
            }
        }
        let mut task_seqs: Vec<u64> = Vec::with_capacity(self.sent_tasks.len());
        for (key, _) in self.sent_tasks.iter() {
            task_seqs.push(*key);
        }
        for key in task_seqs {
            let mut task_item = self.sent_tasks.remove(&key).unwrap();
            let task = task_item.task.take().unwrap();
            retry_with_err!(retry_tx, task, RPC_ERR_CLOSED);
        }
        for tasks_batch_in_second in self.delay_tasks_queue.iter_mut() {
            let mut task_seqs: Vec<u64> = Vec::with_capacity(tasks_batch_in_second.tasks.len());
            for (key, _) in tasks_batch_in_second.tasks.iter() {
                task_seqs.push(*key);
            }
            for key in task_seqs {
                let mut task_item = tasks_batch_in_second.tasks.remove(&key).unwrap();
                let task = task_item.task.take().unwrap();
                retry_with_err!(retry_tx, task, RPC_ERR_CLOSED);
            }
        }
    }

    pub fn check_pending_tasks_empty(&mut self) -> bool {
        loop {
            match self.pending_tasks_recv.try_recv() {
                Ok(task) => {
                    self.got_pending_task(task);
                }
                Err(_) => {
                    break;
                }
            }
        }
        if !self.sent_tasks.is_empty() {
            return false;
        }
        for tasks_batch_in_second in self.delay_tasks_queue.iter() {
            if !tasks_batch_in_second.tasks.is_empty() {
                return false;
            }
        }
        return true;
    }

    // register noti for task.
    #[inline(always)]
    pub async fn reg_task(&self, task: T, wg: Option<WaitGroupGuard>) {
        let _ = self
            .pending_tasks_sender
            .send(RpcClientTaskItem { task: Some(task), _upstream: wg })
            .await;
    }

    // stop register.
    pub fn stop_reg_task(&mut self) {
        self.reg_stopped_flag.store(true, Ordering::Release);
    }

    pub async fn take_task(&mut self, seq: u64) -> Option<RpcClientTaskItem<T>> {
        // ping resp won't readh here
        if seq < self.min_delay_seq {
            return None; // Task is already timeouted by us
        }
        if seq > self.processed_seq {
            let f = WaitRegTaskFuture { noti: self, target_seq: seq };
            if f.await.is_err() {
                return None;
            }
        }

        if let Some(_removed_task) = self.sent_tasks.remove(&seq) {
            return Some(_removed_task);
        }
        for tasks_batch_in_second in self.delay_tasks_queue.iter_mut() {
            if let Some(_task) = tasks_batch_in_second.tasks.remove(&seq) {
                return Some(_task);
            }
        }
        return None;
    }

    #[inline]
    pub fn poll_sent_task<'a>(&mut self, ctx: &mut Context) -> bool {
        let mut got = false;
        // Need to poll_item in order to register waker (HYDROFS-757)
        loop {
            match self.pending_tasks_recv.poll_item(ctx, &mut self.sent_task_waker) {
                Ok(_task) => {
                    self.got_pending_task(_task);
                    got = true;
                    continue;
                }
                Err(_e) => break, // empty or disconnect
            }
        }
        got
    }

    // return None if task is store in sent_tasks
    #[inline]
    fn got_pending_task(&mut self, task_item: RpcClientTaskItem<T>) {
        self.pending_task_count.fetch_sub(1, Ordering::SeqCst);
        let t = task_item.task.as_ref().unwrap();
        let task_seq = t.seq();
        self.processed_seq = task_seq;
        self.sent_tasks.insert(task_seq, task_item);
    }

    pub fn adjust_task_queue(
        &mut self, retry_tx: Option<&mpsc::TxUnbounded<Option<RetryTaskInfo<T>>>>,
    ) {
        // 1. move wait_confirmed to overtime
        let mut tasks_batch_in_second = FxHashMap::default();
        swap(&mut self.sent_tasks, &mut tasks_batch_in_second);

        // 2.notify req with timeout err
        self.delay_tasks_queue.push_front(DelayTasksBatch { tasks: tasks_batch_in_second });

        if self.delay_tasks_queue.len() > self.task_timeout {
            let real_timeout = self.delay_tasks_queue.pop_back().unwrap();
            if !real_timeout.tasks.is_empty() {
                let mut min_seq = 0;
                for (_seq, mut task_item) in real_timeout.tasks {
                    let _task = task_item.task.take().unwrap();
                    let seq = _task.seq();
                    if min_seq == 0 {
                        min_seq = seq;
                    } else {
                        if min_seq > seq {
                            min_seq = seq;
                        }
                    }
                    warn!(
                        "task {} is timeout on client={}:{}",
                        _task, self.server_id, self.client_id
                    );
                    retry_with_err!(retry_tx, _task, RpcError::Rpc(ERR_TIMEOUT));
                }
                self.min_delay_seq = min_seq;
            }
        }
    }
}

struct WaitRegTaskFuture<'a, T>
where
    T: RpcClientTask,
{
    noti: &'a mut RpcClientTaskNotifier<T>,
    target_seq: u64,
}

impl<'a, T> Future for WaitRegTaskFuture<'a, T>
where
    T: RpcClientTask,
{
    type Output = Result<(), ()>;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        let mut _self = self.get_mut();
        if _self.noti.processed_seq >= _self.target_seq {
            return Poll::Ready(Ok(()));
        }
        if _self.noti.reg_stopped_flag.load(Ordering::Acquire) {
            return Poll::Ready(Err(()));
        }
        if _self.noti.poll_sent_task(ctx) {
            if _self.noti.processed_seq >= _self.target_seq {
                return Poll::Ready(Ok(()));
            }
        }
        Poll::Pending
    }
}
