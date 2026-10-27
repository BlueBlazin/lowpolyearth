//! Browser async runtime: one reactor worker drives every future, and a pool
//! of blocking workers runs `spawn_blocking` closures.
//!
//! The reactor worker never blocks, so JavaScript promises (fetch, timers)
//! resolve through its event loop. Wakers are thread-safe; a task woken from
//! another worker is polled again on the reactor. Code on other workers may
//! wait for tasks with [`runtime::Runtime::block_on`], which parks the caller.

use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::future::{AbortHandle, Abortable, Aborted};
use futures_util::task::AtomicWaker;
use futures_util::{FutureExt, StreamExt};
use std::any::Any;
use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

type Task = Pin<Box<dyn Future<Output = ()> + Send>>;

enum Message {
    Spawn(Task),
    /// Runs on the reactor so the closure can create `!Send` JavaScript
    /// futures and hand them to `spawn_local`.
    Local(Box<dyn FnOnce() + Send>),
}

thread_local! {
    static ON_REACTOR: Cell<bool> = const { Cell::new(false) };
}

fn reactor() -> &'static UnboundedSender<Message> {
    static REACTOR: OnceLock<UnboundedSender<Message>> = OnceLock::new();
    REACTOR.get_or_init(|| {
        let (sender, receiver) = unbounded();
        crate::web::thread::spawn_worker(
            "vearth-reactor",
            1 << 20,
            true,
            Box::new(move || run_reactor(receiver)),
        )
        .expect("reactor worker");
        sender
    })
}

fn run_reactor(mut receiver: UnboundedReceiver<Message>) {
    ON_REACTOR.set(true);
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(message) = receiver.next().await {
            match message {
                Message::Spawn(task) => wasm_bindgen_futures::spawn_local(task),
                Message::Local(start) => start(),
            }
        }
    });
}

fn send(message: Message) {
    reactor()
        .unbounded_send(message)
        .unwrap_or_else(|_| panic!("reactor worker stopped"));
}

/// Completion state shared by a task and its [`task::JoinHandle`].
struct Completion<T> {
    result: Mutex<Option<Result<T, task::JoinError>>>,
    finished: AtomicBool,
    waker: AtomicWaker,
}

impl<T> Completion<T> {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            result: Mutex::new(None),
            finished: AtomicBool::new(false),
            waker: AtomicWaker::new(),
        })
    }

    fn complete(&self, result: Result<T, task::JoinError>) {
        *self.result.lock().unwrap() = Some(result);
        self.finished.store(true, Ordering::Release);
        self.waker.wake();
    }
}

fn outcome<T>(
    result: Result<Result<T, Aborted>, Box<dyn Any + Send>>,
) -> Result<T, task::JoinError> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(Aborted)) => Err(task::JoinError::cancelled()),
        Err(payload) => Err(task::JoinError::panic(payload)),
    }
}

/// Runs a `Send` future on the reactor worker.
pub fn spawn<F>(future: F) -> task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let completion = Completion::new();
    let (abort, registration) = AbortHandle::new_pair();
    let done = completion.clone();
    send(Message::Spawn(Box::pin(async move {
        let result = AssertUnwindSafe(Abortable::new(future, registration))
            .catch_unwind()
            .await;
        done.complete(outcome(result));
    })));
    task::JoinHandle::new(completion, Some(abort))
}

/// Builds a `!Send` future on the reactor worker (for example one awaiting a
/// JavaScript promise) and runs it there. The output crosses back to the
/// caller through the returned handle.
pub fn spawn_local_on_reactor<F, Fut>(start: F) -> task::JoinHandle<Fut::Output>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future + 'static,
    Fut::Output: Send + 'static,
{
    let completion = Completion::new();
    let (abort, registration) = AbortHandle::new_pair();
    let done = completion.clone();
    send(Message::Local(Box::new(move || {
        wasm_bindgen_futures::spawn_local(async move {
            let result = AssertUnwindSafe(Abortable::new(start(), registration))
                .catch_unwind()
                .await;
            done.complete(outcome(result));
        });
    })));
    task::JoinHandle::new(completion, Some(abort))
}

type Job = Box<dyn FnOnce() + Send>;

struct BlockingPool {
    state: Mutex<PoolState>,
    available: Condvar,
}

#[derive(Default)]
struct PoolState {
    queue: VecDeque<Job>,
    /// Workers waiting for a job, including those still starting up.
    idle: usize,
    threads: usize,
}

/// Blocking workers in the browser. Each is a Web Worker with its own stack,
/// so the pool is small; queued jobs wait for a free worker.
const MAX_BLOCKING_THREADS: usize = 8;
const BLOCKING_STACK_SIZE: usize = 2 << 20;

fn blocking_pool() -> &'static BlockingPool {
    static POOL: OnceLock<BlockingPool> = OnceLock::new();
    POOL.get_or_init(|| BlockingPool {
        state: Mutex::new(PoolState::default()),
        available: Condvar::new(),
    })
}

fn submit_blocking(job: Job) {
    let pool = blocking_pool();
    let mut state = pool.state.lock().unwrap();
    state.queue.push_back(job);
    if state.idle >= state.queue.len() {
        pool.available.notify_one();
        return;
    }
    if state.threads < MAX_BLOCKING_THREADS {
        let index = state.threads;
        state.threads += 1;
        state.idle += 1;
        drop(state);
        crate::web::thread::spawn_worker(
            &format!("vearth-blocking-{index}"),
            BLOCKING_STACK_SIZE,
            false,
            Box::new(|| blocking_worker(blocking_pool())),
        )
        .expect("blocking worker");
    }
}

fn blocking_worker(pool: &BlockingPool) {
    let mut state = pool.state.lock().unwrap();
    loop {
        match state.queue.pop_front() {
            Some(job) => {
                state.idle -= 1;
                drop(state);
                job();
                state = pool.state.lock().unwrap();
                state.idle += 1;
            }
            None => state = pool.available.wait(state).unwrap(),
        }
    }
}

/// Runs a blocking closure on the blocking worker pool.
pub fn spawn_blocking<F, T>(work: F) -> task::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let completion = Completion::new();
    let done = completion.clone();
    submit_blocking(Box::new(move || {
        let result = catch_unwind(AssertUnwindSafe(work));
        done.complete(result.map_err(task::JoinError::panic));
    }));
    task::JoinHandle::new(completion, None)
}

fn set_timeout(delay: Duration, callback: impl FnOnce() + 'static) {
    let delay_ms = delay.as_secs_f64() * 1e3;
    let global = js_sys::global().unchecked_into::<web_sys::WorkerGlobalScope>();
    global
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            // The callback only stores a flag and wakes a waker.
            Closure::once_into_js(AssertUnwindSafe(callback)).unchecked_ref(),
            delay_ms.ceil().min(f64::from(i32::MAX)) as i32,
        )
        .expect("setTimeout");
}

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

fn park_on<F: Future>(future: F) -> F::Output {
    assert!(
        !ON_REACTOR.get(),
        "block_on on the reactor worker would deadlock its event loop"
    );
    let mut future = std::pin::pin!(future);
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

pub mod task {
    use super::*;

    pub use super::{spawn, spawn_blocking};

    /// A task's panic or cancellation.
    pub struct JoinError {
        panic: Option<Box<dyn Any + Send>>,
    }

    impl JoinError {
        pub(super) fn cancelled() -> Self {
            Self { panic: None }
        }

        pub(super) fn panic(payload: Box<dyn Any + Send>) -> Self {
            Self {
                panic: Some(payload),
            }
        }

        pub fn is_cancelled(&self) -> bool {
            self.panic.is_none()
        }

        pub fn is_panic(&self) -> bool {
            self.panic.is_some()
        }

        pub fn into_panic(self) -> Box<dyn Any + Send> {
            self.panic
                .expect("JoinError is a cancellation, not a panic")
        }
    }

    impl fmt::Display for JoinError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match &self.panic {
                None => f.write_str("task was cancelled"),
                Some(payload) => {
                    let message = payload
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("non-string payload");
                    write!(f, "task panicked: {message}")
                }
            }
        }
    }

    impl fmt::Debug for JoinError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "JoinError({self})")
        }
    }

    impl std::error::Error for JoinError {}

    /// Awaitable task result. Dropping it detaches the task.
    pub struct JoinHandle<T> {
        completion: Arc<Completion<T>>,
        abort: Option<AbortHandle>,
    }

    impl<T> JoinHandle<T> {
        pub(super) fn new(completion: Arc<Completion<T>>, abort: Option<AbortHandle>) -> Self {
            Self { completion, abort }
        }

        /// Cancels an async task at its next poll. Blocking closures, like
        /// Tokio's, run to completion once started.
        pub fn abort(&self) {
            if let Some(abort) = &self.abort {
                abort.abort();
            }
        }

        pub fn is_finished(&self) -> bool {
            self.completion.finished.load(Ordering::Acquire)
        }
    }

    impl<T> Future for JoinHandle<T> {
        type Output = Result<T, JoinError>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            self.completion.waker.register(cx.waker());
            match self.completion.result.lock().unwrap().take() {
                Some(result) => Poll::Ready(result),
                None => Poll::Pending,
            }
        }
    }

    /// A set of spawned tasks, joined in completion order. Dropping the set
    /// aborts the tasks still in it.
    pub struct JoinSet<T> {
        tasks: Vec<JoinHandle<T>>,
    }

    impl<T> Default for JoinSet<T> {
        fn default() -> Self {
            Self { tasks: Vec::new() }
        }
    }

    impl<T: Send + 'static> JoinSet<T> {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn spawn<F>(&mut self, future: F)
        where
            F: Future<Output = T> + Send + 'static,
        {
            self.tasks.push(spawn(future));
        }

        pub fn len(&self) -> usize {
            self.tasks.len()
        }

        pub fn is_empty(&self) -> bool {
            self.tasks.is_empty()
        }

        /// Waits for the next task to finish; `None` once the set is empty.
        pub async fn join_next(&mut self) -> Option<Result<T, JoinError>> {
            if self.tasks.is_empty() {
                return None;
            }
            std::future::poll_fn(|cx| {
                for index in 0..self.tasks.len() {
                    if let Poll::Ready(result) = Pin::new(&mut self.tasks[index]).poll(cx) {
                        self.tasks.swap_remove(index);
                        return Poll::Ready(Some(result));
                    }
                }
                Poll::Pending
            })
            .await
        }

        pub fn abort_all(&mut self) {
            for task in &self.tasks {
                task.abort();
            }
        }
    }

    impl<T> Drop for JoinSet<T> {
        fn drop(&mut self) {
            for task in &self.tasks {
                task.abort();
            }
        }
    }

    /// Yields once to other reactor tasks.
    pub async fn yield_now() {
        let mut yielded = false;
        std::future::poll_fn(|cx| {
            if yielded {
                Poll::Ready(())
            } else {
                yielded = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        })
        .await
    }
}

pub mod time {
    use super::*;
    use crate::time::Instant;

    pub use std::time::Duration;

    struct Timer {
        fired: AtomicBool,
        waker: AtomicWaker,
    }

    /// Completes at a deadline. The timer is a JavaScript timeout on the
    /// reactor, so a `Sleep` may be polled from any worker.
    pub struct Sleep {
        deadline: Instant,
        timer: Option<Arc<Timer>>,
    }

    pub fn sleep(duration: Duration) -> Sleep {
        Sleep {
            deadline: Instant::now() + duration,
            timer: None,
        }
    }

    impl Future for Sleep {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Poll::Ready(());
            }
            match &self.timer {
                Some(timer) => {
                    timer.waker.register(cx.waker());
                    if timer.fired.load(Ordering::Acquire) {
                        // JavaScript timers may fire early by clock rounding;
                        // rearm for the remainder.
                        self.timer = None;
                        cx.waker().wake_by_ref();
                    }
                }
                None => {
                    let timer = Arc::new(Timer {
                        fired: AtomicBool::new(false),
                        waker: AtomicWaker::new(),
                    });
                    timer.waker.register(cx.waker());
                    let weak = Arc::downgrade(&timer);
                    send(Message::Local(Box::new(move || {
                        set_timeout(remaining, move || {
                            if let Some(timer) = weak.upgrade() {
                                timer.fired.store(true, Ordering::Release);
                                timer.waker.wake();
                            }
                        })
                    })));
                    self.timer = Some(timer);
                }
            }
            Poll::Pending
        }
    }

    pub mod error {
        /// A [`super::timeout`] deadline passed first.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Elapsed(pub(super) ());

        impl std::fmt::Display for Elapsed {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("deadline has elapsed")
            }
        }

        impl std::error::Error for Elapsed {}
    }

    pub struct Timeout<F> {
        future: Pin<Box<F>>,
        sleep: Sleep,
    }

    pub fn timeout<F: Future>(duration: Duration, future: F) -> Timeout<F> {
        Timeout {
            future: Box::pin(future),
            sleep: sleep(duration),
        }
    }

    impl<F: Future> Future for Timeout<F> {
        type Output = Result<F::Output, error::Elapsed>;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if let Poll::Ready(output) = self.future.as_mut().poll(cx) {
                return Poll::Ready(Ok(output));
            }
            match Pin::new(&mut self.sleep).poll(cx) {
                Poll::Ready(()) => Poll::Ready(Err(error::Elapsed(()))),
                Poll::Pending => Poll::Pending,
            }
        }
    }
}

pub mod runtime {
    use super::*;

    /// Accepts Tokio's builder options; every browser runtime shares the
    /// reactor worker and blocking pool.
    #[derive(Debug, Default)]
    pub struct Builder(());

    impl Builder {
        pub fn new_multi_thread() -> Self {
            Self(())
        }

        pub fn new_current_thread() -> Self {
            Self(())
        }

        pub fn worker_threads(&mut self, _threads: usize) -> &mut Self {
            self
        }

        pub fn max_blocking_threads(&mut self, _threads: usize) -> &mut Self {
            self
        }

        pub fn thread_name(&mut self, _name: impl Into<String>) -> &mut Self {
            self
        }

        pub fn enable_time(&mut self) -> &mut Self {
            self
        }

        pub fn enable_io(&mut self) -> &mut Self {
            self
        }

        pub fn enable_all(&mut self) -> &mut Self {
            self
        }

        pub fn build(&mut self) -> std::io::Result<Runtime> {
            Ok(Runtime { handle: Handle(()) })
        }
    }

    pub struct Runtime {
        handle: Handle,
    }

    impl Runtime {
        pub fn handle(&self) -> &Handle {
            &self.handle
        }

        pub fn spawn<F>(&self, future: F) -> task::JoinHandle<F::Output>
        where
            F: Future + Send + 'static,
            F::Output: Send + 'static,
        {
            spawn(future)
        }

        pub fn spawn_blocking<F, T>(&self, work: F) -> task::JoinHandle<T>
        where
            F: FnOnce() -> T + Send + 'static,
            T: Send + 'static,
        {
            spawn_blocking(work)
        }

        /// Parks the calling worker until `future` completes. Must not be
        /// called on the reactor worker.
        pub fn block_on<F: Future>(&self, future: F) -> F::Output {
            park_on(future)
        }
    }

    #[derive(Clone, Debug)]
    pub struct Handle(());

    impl Handle {
        pub fn current() -> Self {
            Self(())
        }

        pub fn spawn<F>(&self, future: F) -> task::JoinHandle<F::Output>
        where
            F: Future + Send + 'static,
            F::Output: Send + 'static,
        {
            spawn(future)
        }

        pub fn spawn_blocking<F, T>(&self, work: F) -> task::JoinHandle<T>
        where
            F: FnOnce() -> T + Send + 'static,
            T: Send + 'static,
        {
            spawn_blocking(work)
        }

        pub fn block_on<F: Future>(&self, future: F) -> F::Output {
            park_on(future)
        }
    }
}
