/*
* Copyright 2019 Comcast Cable Communications Management, LLC
*
* Licensed under the Apache License, Version 2.0 (the "License");
* you may not use this file except in compliance with the License.
* You may obtain a copy of the License at
*
* http://www.apache.org/licenses/LICENSE-2.0
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific language governing permissions and
* limitations under the License.
*
* SPDX-License-Identifier: Apache-2.0
*/

use crate::dpdk::{CoreId, Mempool, MempoolMap, MEMPOOL};
use crate::ffi;
use crate::{debug, error, info};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use thiserror::Error;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc as tokio_mpsc;
use tokio::sync::oneshot;
use tokio::task::LocalSet;

/// A boxed future that does not need to be `Send`. It is always spawned onto
/// the thread that created it.
pub(crate) type LocalTask = Pin<Box<dyn Future<Output = ()>>>;

/// A job that creates and returns a `LocalTask` when executed on the target
/// core. The job itself must be `Send` so it can be queued from the master
/// thread, but the task it creates runs locally on the target core.
pub(crate) type Job = Box<dyn FnOnce() -> LocalTask + Send>;

/// A wrapper around the job channel sender. The send error is mapped into
/// an anyhow error here because `Job` is not `Sync`, so the implicit
/// `From<SendError<Job>>` conversion used by `?` does not apply.
#[derive(Debug, Clone)]
pub(crate) struct JobSender(tokio_mpsc::UnboundedSender<Job>);

impl JobSender {
    pub(crate) fn send(&self, job: Job) -> Result<()> {
        self.0
            .send(job)
            .map_err(|_| anyhow::Error::msg("core executor job channel is closed"))
    }
}

/// A sync-channel based park handle.
///
/// This is designed to be a single use handle. We only need to park the
/// core one time at initialization time. Once unparked, we will never
/// park the core again.
pub(crate) struct Park {
    core_id: CoreId,
    sender: SyncSender<()>,
    receiver: Receiver<()>,
}

impl Park {
    fn new(core_id: CoreId) -> Self {
        let (sender, receiver) = mpsc::sync_channel(0);
        Park {
            core_id,
            sender,
            receiver,
        }
    }

    fn unpark(&self) -> Unpark {
        Unpark {
            core_id: self.core_id,
            sender: self.sender.clone(),
        }
    }

    fn park(&self) {
        if let Err(err) = self.receiver.recv() {
            // we are not expecting failures, but we will log it in case.
            error!(message = "park failed.", core=?self.core_id, ?err);
        }
    }
}

/// A sync-channel based unpark handle.
///
/// This is designed to be a single use handle. We will unpark a core one
/// time after all initialization completes. Do not reinvoke this.
pub(crate) struct Unpark {
    core_id: CoreId,
    sender: SyncSender<()>,
}

impl Unpark {
    pub(crate) fn unpark(&self) {
        if let Err(err) = self.sender.send(()) {
            // we are not expecting failures, but we will log it in case.
            error!(message = "unpark failed.", core=?self.core_id, ?err);
        }
    }
}

/// A tokio oneshot channel based shutdown mechanism.
pub(crate) struct Shutdown {
    receiver: oneshot::Receiver<()>,
}

impl Shutdown {
    fn new(core_id: CoreId) -> (Self, ShutdownTrigger) {
        let (sender, receiver) = oneshot::channel();
        let shutdown = Shutdown { receiver };
        let trigger = ShutdownTrigger { core_id, sender };
        (shutdown, trigger)
    }

    fn into_task(self) -> impl Future {
        self.receiver
    }
}

/// A sync-channel based shutdown trigger to terminate a background thread.
pub(crate) struct ShutdownTrigger {
    core_id: CoreId,
    sender: oneshot::Sender<()>,
}

impl ShutdownTrigger {
    pub(crate) fn shutdown(self) {
        if let Err(err) = self.sender.send(()) {
            // we are not expecting failures, but we will log it in case.
            error!(message = "shutdown failed.", core=?self.core_id, ?err);
        }
    }
}

/// A abstraction used to interact with the master/main thread.
///
/// The runtime drives the main loop on the current thread. Pipelines are
/// installed onto the master core through the job queue of the master
/// core's `CoreExecutor`. The `local` set holds the non-Send pipeline tasks,
/// which are driven by the runtime's `block_on` during the main loop.
pub(crate) struct MasterExecutor {
    pub(crate) runtime: Runtime,
    pub(crate) local: LocalSet,
}

/// A thread/core abstraction used to interact with a background thread
/// from the master/main thread.
///
/// When a background thread is first spawned, it is parked and waiting for
/// tasks. Pipelines are installed by sending jobs through the `install`
/// channel; each job runs on the target core and spawns its pipeline onto
/// the core's local task set. Use `unpark` when the tasks are ready to
/// execute.
///
/// The master thread also has an associated `CoreExecutor`, but `unpark`
/// won't do anything because the thread is not parked. Jobs can be sent to
/// it through the `install` channel just like a background thread.
pub(crate) struct CoreExecutor {
    pub(crate) install: JobSender,
    pub(crate) unpark: Option<Unpark>,
    pub(crate) shutdown: Option<ShutdownTrigger>,
    pub(crate) join: Option<JoinHandle<()>>,
}

/// Drains the job queue and spawns each job's task onto the local set. The
/// pump task always runs on the thread that owns the `LocalSet`, so the
/// (possibly non-Send) futures created by the jobs are guaranteed to be
/// created on the core that runs them.
async fn pump(mut jobs: tokio_mpsc::UnboundedReceiver<Job>) {
    while let Some(job) = jobs.recv().await {
        tokio::task::spawn_local(job());
    }
}

/// Core errors.
#[derive(Debug, Error)]
pub(crate) enum CoreError {
    /// Core is not found.
    #[error("{0:?} is not found.")]
    NotFound(CoreId),

    /// Core is not assigned to any ports.
    #[error("{0:?} is not assigned to any ports.")]
    NotAssigned(CoreId),
}

/// Map of all the core handles.
pub(crate) struct CoreMap {
    pub(crate) master_core: MasterExecutor,
    pub(crate) cores: HashMap<CoreId, CoreExecutor>,
}

/// By default, raw pointers do not implement `Send`. We need a simple
/// wrapper so we can send the mempool pointer to a background thread and
/// assigned it to that thread. Otherwise, we wont' be able to create new
/// `Mbuf`s on the background threads.
struct SendablePtr(*mut ffi::rte_mempool);

unsafe impl std::marker::Send for SendablePtr {}

/// Builder for core map.
pub(crate) struct CoreMapBuilder<'a> {
    app_name: String,
    cores: HashSet<CoreId>,
    master_core: CoreId,
    mempools: MempoolMap<'a>,
}

impl<'a> CoreMapBuilder<'a> {
    pub(crate) fn new() -> Self {
        CoreMapBuilder {
            app_name: String::new(),
            cores: Default::default(),
            master_core: CoreId::new(0),
            mempools: Default::default(),
        }
    }

    pub(crate) fn app_name(&mut self, app_name: &str) -> &mut Self {
        self.app_name = app_name.to_owned();
        self
    }

    pub(crate) fn cores(&mut self, cores: &[CoreId]) -> &mut Self {
        self.cores = cores.iter().cloned().collect();
        self
    }

    pub(crate) fn master_core(&mut self, master_core: CoreId) -> &mut Self {
        self.master_core = master_core;
        self
    }

    pub(crate) fn mempools(&'a mut self, mempools: &'a mut [Mempool]) -> &'a mut Self {
        self.mempools = MempoolMap::new(mempools);
        self
    }

    #[allow(clippy::cognitive_complexity)]
    pub(crate) fn finish(&'a mut self) -> Result<CoreMap> {
        let mut map = HashMap::new();

        // first initializes the master core, which the current running
        // thread should be affinitized to.
        let socket_id = self.master_core.socket_id();
        let mempool = self.mempools.get_raw(socket_id)?;

        let (master_thread, core_executor) = init_master_core(self.master_core, mempool)?;

        // adds the master core to the map. tasks can be spawned onto the
        // master core like any other cores.
        map.insert(self.master_core, core_executor);

        info!("initialized master on {:?}.", self.master_core);

        // the core list may also include the master core, to avoid double
        // init, let's try remove it just in case.
        self.cores.remove(&self.master_core);

        // next initializes all the cores other than the master core
        for &core_id in self.cores.iter() {
            // finds the mempool that matches the core's socket, and wraps the
            // reference in a sendable pointer because we are sending it to
            // a background thread
            let socket_id = core_id.socket_id();
            let mempool = self.mempools.get_raw(socket_id)?;
            let ptr = SendablePtr(mempool);

            // creates a synchronous channel so we can retrieve the executor for
            // the background core.
            let (sender, receiver) = mpsc::sync_channel(0);

            // spawns a new background thread and initializes a core executor on
            // that thread.
            let join = thread::Builder::new()
                .name(format!("{}-{:?}", self.app_name, core_id))
                .spawn(move || {
                    debug!("spawned background thread {:?}.", thread::current().id());

                    match init_background_core(core_id, ptr.0) {
                        Ok((runtime, local, park, shutdown, executor)) => {
                            info!("initialized thread on {:?}.", core_id);

                            // sends the executor back to the master core. it's safe to unwrap
                            // the result because the receiving end is guaranteed to be in scope.
                            sender.send(Ok(executor)).unwrap();

                            info!("parking {:?}.", core_id);

                            // sleeps the thread for now since there's nothing to be done yet.
                            // once new tasks are queued, the master core can unpark this and
                            // let the execution continue.
                            park.park();

                            info!("unparked {:?}.", core_id);

                            // once the thread wakes up, we will run all the queued jobs and
                            // wait until a shutdown is triggered from the master core.
                            let _ = runtime.block_on(local.run_until(shutdown.into_task()));

                            info!("unblocked {:?}.", core_id);
                        }
                        // propogates the error back to the master core.
                        Err(err) => sender.send(Err(err)).unwrap(),
                    }
                })?;

            // blocks and waits for the background thread to finish initialize.
            // when done, we add the executor to the map.
            let mut executor = receiver.recv().unwrap()?;
            executor.join = Some(join);
            map.insert(core_id, executor);
        }

        Ok(CoreMap {
            master_core: master_thread,
            cores: map,
        })
    }
}

fn init_master_core(
    id: CoreId,
    mempool: *mut ffi::rte_mempool,
) -> Result<(MasterExecutor, CoreExecutor)> {
    // affinitize the running thread to this core.
    id.set_thread_affinity()?;

    // sets the mempool
    MEMPOOL.with(|tls| tls.set(mempool));

    // starts a single-threaded runtime with the IO and time drivers enabled,
    // so we can receive signals and schedule timed tasks on the master core.
    let runtime = Builder::new_current_thread().enable_all().build()?;

    // the local set holds the non-Send pipeline tasks; it is driven by the
    // main thread inside the runtime's `block_on`.
    let local = LocalSet::new();

    // starts the job queue, we can use this handle to install pipelines
    // onto this core from the master core.
    let (install, jobs) = tokio_mpsc::unbounded_channel::<Job>();
    local.spawn_local(pump(jobs));

    let main = MasterExecutor { runtime, local };

    let executor = CoreExecutor {
        install: JobSender(install),
        unpark: None,
        shutdown: None,
        join: None,
    };

    Ok((main, executor))
}

fn init_background_core(
    id: CoreId,
    mempool: *mut ffi::rte_mempool,
) -> Result<(Runtime, LocalSet, Park, Shutdown, CoreExecutor)> {
    // affinitize the running thread to this core.
    id.set_thread_affinity()?;

    // sets the mempool
    MEMPOOL.with(|tls| tls.set(mempool));

    // starts a single-threaded runtime with the IO and time drivers enabled,
    // so we can schedule timed tasks on this core.
    let runtime = Builder::new_current_thread().enable_all().build()?;

    // the local set holds the non-Send pipeline tasks; it is driven by this
    // thread inside the runtime's `block_on`.
    let local = LocalSet::new();

    // starts the job queue, the master core can use this handle to install
    // pipelines onto this core.
    let (install, jobs) = tokio_mpsc::unbounded_channel::<Job>();
    local.spawn_local(pump(jobs));

    // problem with running the executor eagerly is the tasks would start
    // executing before the ports are started. so instead, we use a channel
    // based park mechanism to block the thread from further execution until
    // we are ready to proceed.
    let park = Park::new(id);

    // shutdown handle for the core.
    let (shutdown, trigger) = Shutdown::new(id);

    let executor = CoreExecutor {
        install: JobSender(install),
        unpark: Some(park.unpark()),
        shutdown: Some(trigger),
        join: None,
    };

    Ok((runtime, local, park, shutdown, executor))
}
