//! Loads plug-ins on worker threads.
//!
//! [`Loads::start`] hands its jobs to a pool of at most [`MAX_WORKERS`] threads and returns at
//! once. The web build has no threads, so there it runs the jobs one after the other before it
//! returns. Each job runs inside a panic boundary: a job that panics fails with [`Error::Failed`],
//! and the launch marker stops naming its item ([`crate::safe_mode::loading`]), as when it loads.
//! A job that takes [`LOAD_DEADLINE`] or longer fails too, and the marker names its item until its
//! thread returns; while it is still running, its thread is left to finish on its own, and another worker
//! takes the jobs after it, up to twice as many threads as were started. While
//! [`MAX_STUCK_THREADS`] threads in the process still run jobs past their deadline, no load starts
//! a thread. When no thread is left to take them, the jobs not started fail.
//!
//! The caller takes the outcomes in the order of the jobs, whatever order they finish in
//! ([`Loads::ready`]), so it installs them in the same order on every launch. A job starts only
//! while fewer than twice the workers' count of outcomes before it are waiting to be taken or are
//! still held by the caller ([`Loads::release`]), so the loaded modules held at once stay bounded
//! however many jobs there are.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use web_time::Instant;

use crate::safe_mode::{self, Item};
use crate::{Error, Result};

/// The most worker threads one [`Loads`] starts, besides those that replace a worker whose job
/// passed [`LOAD_DEADLINE`].
pub const MAX_WORKERS: usize = 4;

/// How long one job may take; one that takes longer fails as not finished.
pub const LOAD_DEADLINE: Duration = Duration::from_secs(10);

/// How often a wait looks at the deadlines while no job finishes.
const TICK: Duration = Duration::from_millis(50);

/// The most loader threads in the process still running a job past its deadline. While that many
/// are, no [`Loads`] starts another thread, so loads started again and again on files whose reads
/// never return don't pile up threads.
pub const MAX_STUCK_THREADS: usize = 2 * MAX_WORKERS;

/// Loader threads in the process running a job past its deadline, across every [`Loads`].
static STUCK_THREADS: AtomicUsize = AtomicUsize::new(0);

/// The [`Error::Failed`] message of a job that failed because no thread was left to run it.
pub const NO_THREAD: &str = "no thread was left to load it";

/// One plug-in to load: the item the launch marker names while it loads, and the work, which runs
/// on a worker thread.
pub struct Job<T> {
    pub item: Option<Item>,
    pub work: Box<dyn FnOnce() -> Result<T> + Send>,
}

impl<T> Job<T> {
    pub fn new(item: Option<Item>, work: impl FnOnce() -> Result<T> + Send + 'static) -> Self {
        Job { item, work: Box::new(work) }
    }
}

/// The time since a fixed start, for the deadlines.
type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

/// A job's outcome as its worker sends it: the job, its result and how long it took.
type Done<T> = (usize, Result<T>, Duration);

/// What the workers and [`Loads`] share.
struct State<T> {
    /// The jobs not started yet, in order.
    queue: VecDeque<(usize, Job<T>)>,
    /// When each job started, and whether it has finished or passed its deadline.
    started: Vec<Option<Duration>>,
    finished: Vec<bool>,
    expired: Vec<bool>,
    /// Jobs whose index is below this may start.
    limit: usize,
    /// Worker threads alive, and how many of them run a job past its deadline.
    workers: usize,
    stuck: usize,
}

struct Pool<T> {
    state: Mutex<State<T>>,
    /// Told when a job may start, or when the queue is emptied.
    wake: Condvar,
}

impl<T> Pool<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Counts a worker as alive until its thread ends, however it ends.
struct Alive<T>(Arc<Pool<T>>);

impl<T> Drop for Alive<T> {
    fn drop(&mut self) {
        self.0.lock().workers -= 1;
    }
}

/// Jobs loading on worker threads ([`Loads::start`]).
pub struct Loads<T: Send + 'static> {
    pool: Arc<Pool<T>>,
    results: Mutex<Receiver<Done<T>>>,
    send: Sender<Done<T>>,
    outcomes: Vec<Option<Result<T>>>,
    /// The outcomes before this one were taken ([`Loads::ready`]).
    next: usize,
    /// Of those, the ones the caller is done with ([`Loads::release`]).
    released: usize,
    /// How many jobs past `next` may start.
    window: usize,
    clock: Clock,
    deadline: Duration,
    /// The workers [`Loads::begin`] starts, and whether it has.
    workers: usize,
    begun: bool,
    /// The process's count of stuck loader threads ([`STUCK_THREADS`]), and its most.
    stuck_threads: &'static AtomicUsize,
    max_stuck: usize,
    /// Tests: no thread can be started from now on.
    #[cfg(test)]
    pub(crate) refuse_threads: bool,
}

impl<T: Send + 'static> Loads<T> {
    /// Starts `jobs` on `min(available parallelism, MAX_WORKERS)` threads.
    pub fn start(jobs: Vec<Job<T>>) -> Self {
        let mut loads = Self::deferred(jobs);
        loads.begin();
        loads
    }

    /// `jobs` to start on `min(available parallelism, MAX_WORKERS)` threads once [`Loads::begin`]
    /// is called (or something waits for them).
    pub fn deferred(jobs: Vec<Job<T>>) -> Self {
        Self::deferred_on(jobs, std::thread::available_parallelism().map_or(1, |n| n.get()).min(MAX_WORKERS))
    }

    /// [`Loads::deferred`] on `workers` threads (at least one).
    pub(crate) fn deferred_on(jobs: Vec<Job<T>>, workers: usize) -> Self {
        let origin = Instant::now();
        Self::with(jobs, workers, LOAD_DEADLINE, Arc::new(move || origin.elapsed()))
    }

    /// Starts the jobs, unless they started already.
    pub fn begin(&mut self) {
        if self.begun {
            return;
        }
        self.begun = true;
        if cfg!(target_arch = "wasm32") {
            // The web build has no threads: each job runs here, in order.
            let jobs: Vec<_> = self.pool.lock().queue.drain(..).collect();
            for (i, job) in jobs {
                // A job can't be stopped here, but one that took past its deadline fails as on
                // the desktop.
                let at = (self.clock)();
                let r = run_job(job);
                let took = (self.clock)().saturating_sub(at);
                self.record((i, r, took));
            }
        } else {
            for _ in 0..self.workers.min(self.outcomes.len()) {
                self.spawn_worker();
            }
        }
    }

    #[cfg(test)]
    fn start_with(jobs: Vec<Job<T>>, workers: usize, deadline: Duration, clock: Clock) -> Self {
        let mut loads = Self::with(jobs, workers, deadline, clock);
        loads.begin();
        loads
    }

    fn with(jobs: Vec<Job<T>>, workers: usize, deadline: Duration, clock: Clock) -> Self {
        let n = jobs.len();
        let workers = workers.max(1);
        let window = workers * 2;
        let state = State {
            queue: jobs.into_iter().enumerate().collect(),
            started: vec![None; n],
            finished: vec![false; n],
            expired: vec![false; n],
            limit: window,
            workers: 0,
            stuck: 0,
        };
        let (send, results) = mpsc::channel();
        Loads {
            pool: Arc::new(Pool { state: Mutex::new(state), wake: Condvar::new() }),
            results: Mutex::new(results),
            send,
            outcomes: (0..n).map(|_| None).collect(),
            next: 0,
            released: 0,
            window,
            clock,
            deadline,
            workers,
            begun: false,
            stuck_threads: &STUCK_THREADS,
            max_stuck: MAX_STUCK_THREADS,
            #[cfg(test)]
            refuse_threads: false,
        }
    }

    /// Starts a thread that takes jobs until none are left → whether it started.
    fn spawn_worker(&self) -> bool {
        #[cfg(test)]
        if self.refuse_threads {
            return false;
        }
        if self.stuck_threads.load(Ordering::SeqCst) >= self.max_stuck {
            log::warn!("not starting a thread to load plug-ins: {} loader threads are still running loads past their deadline", self.max_stuck);
            return false;
        }
        let pool = self.pool.clone();
        let send = self.send.clone();
        let clock = self.clock.clone();
        let target = self.workers;
        let stuck_threads = self.stuck_threads;
        self.pool.lock().workers += 1;
        let spawned = std::thread::Builder::new().name("plug-in loader".into()).spawn(move || {
            let alive = Alive(pool);
            loop {
                let (i, job, at) = {
                    let mut s = alive.0.lock();
                    loop {
                        match s.queue.front() {
                            None => return,
                            Some((i, _)) if *i < s.limit => break,
                            Some(_) => s = alive.0.wake.wait(s).unwrap_or_else(|e| e.into_inner()),
                        }
                    }
                    let Some((i, job)) = s.queue.pop_front() else { return };
                    let at = clock();
                    s.started[i] = Some(at);
                    (i, job, at)
                };
                let r = run_job(job);
                // A worker whose job passed its deadline and was replaced ends once the job
                // returns, unless the workers able to take jobs are fewer than started. The time
                // the job took is read while the job is marked finished, so a deadline check that
                // finds it unfinished counts a time that its own outcome reaches too.
                let (took, retire) = {
                    let mut s = alive.0.lock();
                    let took = clock().saturating_sub(at);
                    s.finished[i] = true;
                    let retire = s.expired[i] && {
                        s.stuck -= 1;
                        stuck_threads.fetch_sub(1, Ordering::SeqCst);
                        s.workers - s.stuck > target
                    };
                    (took, retire)
                };
                let _ = send.send((i, r, took));
                if retire {
                    return;
                }
            }
        });
        if let Err(e) = spawned {
            self.pool.lock().workers -= 1;
            log::warn!("can't start a thread to load plug-ins: {e}");
            return false;
        }
        true
    }

    /// Takes the outcomes that came in and fails the jobs past their deadline → whether every job
    /// has its outcome.
    pub fn poll(&mut self) -> bool {
        loop {
            let next = self.results.lock().unwrap_or_else(|e| e.into_inner()).try_recv();
            let Ok(done) = next else { break };
            self.record(done);
        }
        self.expire();
        self.outcomes.iter().all(Option::is_some)
    }

    /// The outcomes not taken yet that come in order from the last one taken, without waiting
    /// ([`Loads::poll`] first). The jobs after them start as the caller releases them
    /// ([`Loads::release`]).
    pub fn ready(&mut self) -> Vec<Result<T>> {
        self.poll();
        let mut out = Vec::new();
        while let Some(o) = self.outcomes.get_mut(self.next).and_then(Option::take) {
            out.push(o);
            self.next += 1;
        }
        out
    }

    /// The caller is done with `n` more of the outcomes it took (installed or dropped them): as
    /// many more jobs may start, so the outcomes held, here and by the caller, stay at most twice
    /// the workers' count.
    pub fn release(&mut self, n: usize) {
        let released = (self.released + n).min(self.next);
        if released != self.released {
            self.released = released;
            self.pool.lock().limit = released + self.window;
            self.pool.wake.notify_all();
        }
    }

    /// Whether every outcome has been taken.
    pub fn taken(&self) -> bool {
        self.next == self.outcomes.len()
    }

    /// Whether the next outcome in order can be taken ([`Loads::ready`]).
    pub fn next_ready(&mut self) -> bool {
        self.poll();
        self.outcomes.get(self.next).is_some_and(Option::is_some)
    }

    /// Waits until the next outcome in order can be taken, or every one has been. Jobs that have
    /// not been started are started first ([`Loads::begin`]).
    pub fn wait_next(&mut self) {
        self.begin();
        while !self.taken() && !self.next_ready() {
            let next = self.results.lock().unwrap_or_else(|e| e.into_inner()).recv_timeout(TICK);
            match next {
                Ok(done) => self.record(done),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
            }
        }
    }

    /// Waits for every outcome → those not taken yet, in the order of the jobs. The outcomes taken
    /// before ([`Loads::ready`]) count as released, so the jobs after them start.
    pub fn wait(mut self) -> Vec<Result<T>> {
        self.release(self.next - self.released);
        let mut out = Vec::new();
        while !self.taken() {
            self.wait_next();
            let ready = self.ready();
            self.release(ready.len());
            out.extend(ready);
        }
        out
    }

    fn record(&mut self, (i, r, took): Done<T>) {
        let r = if took >= self.deadline { Err(self.too_long()) } else { r };
        // An outcome that comes after the job failed past its deadline is dropped.
        if let Some(slot @ None) = self.outcomes.get_mut(i) {
            *slot = Some(r);
        }
    }

    fn too_long(&self) -> Error {
        let secs = self.deadline.as_secs();
        Error::Failed(format!("did not finish loading within {secs} s"))
    }

    /// Fails the jobs still running past their deadline, starting a worker in place of each one's
    /// while the threads alive are fewer than twice the workers started. When no worker is left to
    /// take the jobs not started (none could be made, or every one is stuck), those fail too.
    fn expire(&mut self) {
        if !self.begun {
            return;
        }
        let mut expired = Vec::new();
        {
            let mut s = self.pool.lock();
            // Read while the workers can't mark a job finished ([`Loads::spawn_worker`]).
            let now = (self.clock)();
            for i in 0..self.outcomes.len() {
                let late = s.started[i].is_some_and(|at| now.saturating_sub(at) >= self.deadline);
                if late && !s.finished[i] && !s.expired[i] && self.outcomes[i].is_none() {
                    s.expired[i] = true;
                    s.stuck += 1;
                    self.stuck_threads.fetch_add(1, Ordering::SeqCst);
                    expired.push(i);
                }
            }
        }
        for i in &expired {
            self.outcomes[*i] = Some(Err(self.too_long()));
        }
        for _ in &expired {
            let room = {
                let s = self.pool.lock();
                !s.queue.is_empty() && s.workers < 2 * self.workers
            };
            if !room || !self.spawn_worker() {
                break;
            }
        }
        // The jobs not started are taken from the queue under the lock that finds no worker able to
        // take them, so a stuck worker that returns can't start one of them in between. Only those
        // fail: a job already started reports its own outcome.
        let stranded: Vec<usize> = {
            let mut s = self.pool.lock();
            if s.workers <= s.stuck { s.queue.drain(..).map(|(i, _)| i).collect() } else { Vec::new() }
        };
        for i in stranded {
            if let Some(o @ None) = self.outcomes.get_mut(i) {
                *o = Some(Err(Error::Failed(NO_THREAD.into())));
            }
        }
    }
}

impl<T: Send + 'static> Drop for Loads<T> {
    /// Jobs not started yet are dropped; the running ones are given until their deadline, so the
    /// launch marker is not left naming a plug-in that was still loading when the app quit.
    fn drop(&mut self) {
        self.pool.lock().queue.clear();
        self.pool.wake.notify_all();
        loop {
            let running = {
                let s = self.pool.lock();
                let now = (self.clock)();
                (0..self.outcomes.len())
                    .any(|i| !s.finished[i] && !s.expired[i] && s.started[i].is_some_and(|at| now.saturating_sub(at) < self.deadline))
            };
            if !running {
                break;
            }
            let next = self.results.lock().unwrap_or_else(|e| e.into_inner()).recv_timeout(TICK);
            if let Ok(done) = next {
                self.record(done);
            }
        }
        // A job still running past its deadline leaves its thread stuck, and the thread counts
        // against the process's stuck threads until the job returns.
        let mut s = self.pool.lock();
        for i in 0..self.outcomes.len() {
            if s.started[i].is_some() && !s.finished[i] && !s.expired[i] {
                s.expired[i] = true;
                s.stuck += 1;
                self.stuck_threads.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
}

/// Runs `job` inside a panic boundary, with the launch marker naming its item while it runs.
fn run_job<T>(job: Job<T>) -> Result<T> {
    let token = job.item.as_ref().map(safe_mode::loading);
    let result = catch_unwind(AssertUnwindSafe(job.work));
    if let Some(t) = token {
        safe_mode::loaded(t);
    }
    match result {
        Ok(r) => r,
        Err(panic) => {
            let what = panic.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| panic.downcast_ref::<String>().cloned());
            Err(match what {
                Some(what) => Error::Failed(format!("panicked while loading: {what}")),
                None => Error::Failed("panicked while loading".into()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::sync::mpsc::sync_channel;

    /// A clock the test moves.
    fn manual() -> (Arc<AtomicU64>, Clock) {
        let ms = Arc::new(AtomicU64::new(0));
        let read = ms.clone();
        (ms, Arc::new(move || Duration::from_millis(read.load(Ordering::SeqCst))))
    }

    /// A job that says it started on `started`, then waits for `go` and returns `v`.
    fn held(v: u32, started: mpsc::SyncSender<u32>, go: Receiver<()>) -> Job<u32> {
        Job::new(None, move || {
            let _ = started.send(v);
            let _ = go.recv();
            Ok(v)
        })
    }

    /// Polls `loads` until `done` says so, at most a million times.
    fn poll_until(loads: &mut Loads<u32>, mut done: impl FnMut(&mut Loads<u32>) -> bool) -> bool {
        (0..1_000_000).any(|_| {
            std::thread::yield_now();
            loads.poll();
            done(loads)
        })
    }

    /// What `rx` receives, looked for 100 million times at most, so a worker that never sends fails the
    /// test instead of holding it. The test fails at once when every sender is gone.
    fn recv_soon<V>(rx: &Receiver<V>) -> V {
        for _ in 0..100_000_000 {
            match rx.try_recv() {
                Ok(v) => return v,
                Err(mpsc::TryRecvError::Disconnected) => panic!("every sender ended without sending"),
                Err(mpsc::TryRecvError::Empty) => std::thread::yield_now(),
            }
        }
        panic!("nothing was received");
    }

    /// The outcomes come back in the order of the jobs when they finish in the reverse order.
    #[test]
    fn outcomes_keep_the_order_of_the_jobs() {
        let (started, starts) = sync_channel(3);
        let mut go = Vec::new();
        let mut jobs = Vec::new();
        for v in 0..3 {
            let (tx, rx) = mpsc::channel();
            go.push(tx);
            jobs.push(held(v, started.clone(), rx));
        }
        let (_, clock) = manual();
        let mut loads = Loads::start_with(jobs, 3, LOAD_DEADLINE, clock);
        for _ in 0..3 {
            recv_soon(&starts);
        }
        // Each job is let go only once the one after it has finished.
        let mut finished = Vec::new();
        for v in (0..3).rev() {
            go[v].send(()).unwrap();
            let done = recv_soon(&loads.results.lock().unwrap());
            finished.push(done.0);
            loads.record(done);
        }
        assert_eq!(finished, vec![2, 1, 0], "they finished in reverse");
        assert_eq!(loads.wait().into_iter().map(|r| r.unwrap()).collect::<Vec<_>>(), vec![0, 1, 2]);
    }

    /// A job that panics fails with the panic's message, and the others load.
    #[test]
    fn a_job_that_panics_fails_alone() {
        let jobs: Vec<Job<u32>> = vec![Job::new(None, || Ok(1)), Job::new(None, || panic!("broken module")), Job::new(None, || Ok(3))];
        let (_, clock) = manual();
        let out = Loads::start_with(jobs, 2, LOAD_DEADLINE, clock).wait();
        assert_eq!(out[0], Ok(1));
        assert!(matches!(&out[1], Err(Error::Failed(m)) if m.contains("panicked") && m.contains("broken module")), "{:?}", out[1]);
        assert_eq!(out[2], Ok(3));
    }

    /// With one worker, a job that never finishes fails at its deadline, and the job after it
    /// still loads, on the worker that takes its place.
    #[test]
    fn a_job_that_hangs_fails_at_its_deadline_and_the_rest_load() {
        let (started, starts) = sync_channel(2);
        let (go, held_until) = mpsc::channel();
        let jobs = vec![held(0, started.clone(), held_until), Job::new(None, || Ok(1))];
        let (ms, clock) = manual();
        let mut loads = Loads::start_with(jobs, 1, Duration::from_secs(10), clock);
        assert_eq!(recv_soon(&starts), 0);
        assert!(!loads.poll(), "the first job is still running and the second waits for a worker");
        ms.store(10_000, Ordering::SeqCst);
        let done = poll_until(&mut loads, |l| l.outcomes.iter().all(Option::is_some));
        go.send(()).unwrap();
        assert!(done, "the job after the one that hangs never got its outcome");
        let out = loads.wait();
        assert!(matches!(&out[0], Err(Error::Failed(m)) if m.contains("did not finish")), "{:?}", out[0]);
        assert_eq!(out[1], Ok(1));
    }

    /// Workers that replace stuck ones stop at twice the workers started: with one worker and two
    /// jobs that never finish, the third job fails instead of a third thread taking it.
    #[test]
    fn replacements_stop_at_twice_the_workers() {
        let (started, starts) = sync_channel(3);
        let (go0, held0) = mpsc::channel();
        let (go1, held1) = mpsc::channel();
        let jobs = vec![held(0, started.clone(), held0), held(1, started, held1), Job::new(None, || Ok(2))];
        let (ms, clock) = manual();
        let mut loads = Loads::start_with(jobs, 1, Duration::from_secs(10), clock);
        assert_eq!(recv_soon(&starts), 0);
        ms.store(10_000, Ordering::SeqCst);
        loads.poll();
        assert_eq!(recv_soon(&starts), 1, "a worker took the place of the stuck one");
        ms.store(20_000, Ordering::SeqCst);
        let done = loads.poll();
        go0.send(()).unwrap();
        go1.send(()).unwrap();
        assert!(done, "every job has its outcome");
        let out = loads.wait();
        assert!(matches!(&out[2], Err(Error::Failed(m)) if m.contains("no thread")), "{:?}", out[2]);
    }

    /// A job that finished but took longer than the deadline fails, however soon it is looked at.
    #[test]
    fn a_job_that_took_too_long_fails_when_it_is_taken() {
        let (ms, clock) = manual();
        let moved = ms.clone();
        let jobs = vec![Job::new(None, move || {
            moved.store(10_000, Ordering::SeqCst);
            Ok(1u32)
        })];
        let out = Loads::start_with(jobs, 1, Duration::from_secs(10), clock).wait();
        assert!(matches!(&out[0], Err(Error::Failed(m)) if m.contains("did not finish")), "{:?}", out[0]);
    }

    /// Only twice the workers' count of jobs past the last outcome taken start: the next one waits
    /// until an outcome is taken.
    #[test]
    fn jobs_past_the_window_wait_until_outcomes_are_taken() {
        let ran = Arc::new(AtomicUsize::new(0));
        let jobs: Vec<Job<u32>> = (0..5)
            .map(|v| {
                let ran = ran.clone();
                Job::new(None, move || {
                    ran.fetch_add(1, Ordering::SeqCst);
                    Ok(v)
                })
            })
            .collect();
        let (_, clock) = manual();
        let mut loads = Loads::start_with(jobs, 1, LOAD_DEADLINE, clock);
        assert!(poll_until(&mut loads, |l| l.outcomes.iter().filter(|o| o.is_some()).count() == 2), "the first two loaded");
        let early = ran.load(Ordering::SeqCst);
        assert_eq!(loads.ready(), vec![Ok(0), Ok(1)]);
        assert_eq!(early, 2, "the third job waited for the first outcomes to be taken");
        assert!(!poll_until(&mut loads, |l| l.outcomes.iter().filter(|o| o.is_some()).count() > 0), "taken but not released");
        assert_eq!(ran.load(Ordering::SeqCst), 2, "the third job waits until the caller is done with the first outcomes");
        loads.release(2);
        assert_eq!(loads.wait(), vec![Ok(2), Ok(3), Ok(4)]);
    }

    /// `wait` after outcomes were taken and not released still gets the rest: it counts the outcomes
    /// taken as released, so the job past the window starts.
    #[test]
    fn wait_after_outcomes_were_taken_gets_the_jobs_past_the_window() {
        let jobs: Vec<Job<u32>> = (0..3).map(|v| Job::new(None, move || Ok(v))).collect();
        let (_, clock) = manual();
        let mut loads = Loads::start_with(jobs, 1, LOAD_DEADLINE, clock);
        assert!(poll_until(&mut loads, |l| l.outcomes.iter().filter(|o| o.is_some()).count() == 2), "the first two loaded");
        assert_eq!(loads.ready(), vec![Ok(0), Ok(1)]);
        let (done, rest) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(loads.wait());
        });
        assert_eq!(recv_soon(&rest), vec![Ok(2)]);
    }

    /// When no worker can take the jobs left, because the only one is stuck past its deadline and
    /// no thread takes its place, those jobs fail instead of being waited for.
    #[test]
    fn jobs_no_worker_can_take_fail() {
        let (started, starts) = sync_channel(1);
        let (go, held_until) = mpsc::channel();
        let jobs = vec![held(0, started, held_until), Job::new(None, || Ok(1))];
        let (ms, clock) = manual();
        let mut loads = Loads::start_with(jobs, 1, Duration::from_secs(10), clock);
        recv_soon(&starts);
        loads.refuse_threads = true;
        ms.store(10_000, Ordering::SeqCst);
        let done = loads.poll();
        go.send(()).unwrap();
        assert!(done, "every job has its outcome");
        let out = loads.wait();
        assert!(matches!(&out[0], Err(Error::Failed(m)) if m.contains("did not finish")), "{:?}", out[0]);
        assert!(matches!(&out[1], Err(Error::Failed(m)) if m.contains("no thread")), "{:?}", out[1]);
    }

    /// A thread stuck past its deadline counts against later loads until its job returns: with
    /// room for one stuck thread in the process, the load after one that left a thread stuck
    /// starts no thread and fails its job, and once the stuck job returns a load runs again.
    #[test]
    fn stuck_threads_count_across_loads_until_they_end() {
        let stuck: &'static AtomicUsize = Box::leak(Box::new(AtomicUsize::new(0)));
        let make = |jobs: Vec<Job<u32>>, clock: Clock| {
            let mut loads = Loads::with(jobs, 1, Duration::from_secs(10), clock);
            loads.stuck_threads = stuck;
            loads.max_stuck = 1;
            loads.begin();
            loads
        };
        let (started, starts) = sync_channel(1);
        let (go, held_until) = mpsc::channel();
        let (ms, clock) = manual();
        let mut first = make(vec![held(0, started, held_until)], clock);
        recv_soon(&starts);
        ms.store(10_000, Ordering::SeqCst);
        assert!(first.poll(), "the stuck job failed at its deadline");
        drop(first);
        let (_, clock) = manual();
        let out = make(vec![Job::new(None, || Ok(1))], clock.clone()).wait();
        assert!(matches!(&out[0], Err(Error::Failed(m)) if m.contains("no thread")), "{:?}", out[0]);
        go.send(()).unwrap();
        assert!((0..1_000_000).any(|_| {
            std::thread::yield_now();
            stuck.load(Ordering::SeqCst) == 0
        }));
        assert_eq!(make(vec![Job::new(None, || Ok(2))], clock).wait(), vec![Ok(2)]);
    }

    /// A load dropped while a job runs past its deadline, before anything looked at the job, still
    /// counts the job's thread as stuck until the job returns.
    #[test]
    fn a_dropped_load_counts_a_job_past_its_deadline_as_stuck() {
        let stuck: &'static AtomicUsize = Box::leak(Box::new(AtomicUsize::new(0)));
        let (started, starts) = sync_channel(1);
        let (go, held_until) = mpsc::channel();
        let (ms, clock) = manual();
        let mut loads = Loads::with(vec![held(0, started, held_until)], 1, Duration::from_secs(10), clock);
        loads.stuck_threads = stuck;
        loads.begin();
        recv_soon(&starts);
        ms.store(10_000, Ordering::SeqCst);
        drop(loads);
        assert_eq!(stuck.load(Ordering::SeqCst), 1, "the stuck thread counts");
        go.send(()).unwrap();
        assert!((0..1_000_000).any(|_| {
            std::thread::yield_now();
            stuck.load(Ordering::SeqCst) == 0
        }));
    }

    /// The launch marker names an item while its job runs, and stops naming it when the job
    /// returns, a job that panicked included.
    #[test]
    fn a_job_names_its_item_until_it_returns_even_when_it_panics() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let ok = Item::file("test-loader-ok.wasm");
        let bad = Item::file("test-loader-panics.wasm");
        let named = |item: &Item| crate::safe_mode::still_loading().iter().any(|i| i.same(item));
        let (ok2, bad2) = (ok.clone(), bad.clone());
        let bad_named = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = bad_named.clone();
        let jobs: Vec<Job<bool>> = vec![
            Job::new(Some(ok.clone()), move || Ok(named(&ok2))),
            Job::new(Some(bad.clone()), move || {
                seen.store(named(&bad2), Ordering::SeqCst);
                panic!("x")
            }),
        ];
        let (_, clock) = manual();
        let outcomes = Loads::start_with(jobs, 2, LOAD_DEADLINE, clock).wait();
        assert!(matches!(outcomes.first(), Some(Ok(true))), "named while it runs");
        assert!(bad_named.load(Ordering::SeqCst), "named while it runs, before it panics");
        assert!(matches!(outcomes.get(1), Some(Err(Error::Failed(m))) if m.contains("panicked while loading")), "the panic is the job's failure");
        let running = crate::safe_mode::still_loading();
        assert!(!running.iter().any(|i| i.same(&bad) || i.same(&ok)), "{running:?}");
    }
}
