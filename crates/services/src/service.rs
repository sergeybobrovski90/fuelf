use anyhow::anyhow;
use tokio::{
    sync::watch,
    task::JoinHandle,
};

pub type Shared<T> = std::sync::Arc<T>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyShared;

pub fn empty_shared() -> Shared<EmptyShared> {
    Shared::new(EmptyShared)
}

#[async_trait::async_trait]
pub trait Service {
    fn start(&self) -> anyhow::Result<()>;

    fn stop(&self) -> bool;

    async fn stop_and_await(&self) -> anyhow::Result<()>;

    fn state(&self) -> State;
}

#[async_trait::async_trait]
pub trait RunnableService: Send + Sync {
    type SharedData: Send + Sync;

    fn shared_data(&self) -> Shared<Self::SharedData>;

    async fn initialize(&mut self) -> anyhow::Result<()>;

    /// `ServiceRunner` calls `run` function until it returns `false`.
    async fn run(&mut self) -> anyhow::Result<bool>;
}

#[derive(Debug, Clone)]
pub enum State {
    NotStarted,
    Started,
    Stopping,
    Stopped,
    StoppedWithError(String),
}

impl State {
    pub fn not_started(&self) -> bool {
        self == &State::NotStarted
    }

    pub fn started(&self) -> bool {
        self == &State::Started
    }

    pub fn stopped(&self) -> bool {
        match self {
            State::Stopped | State::StoppedWithError(_) => true,
            _ => false,
        }
    }
}

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::NotStarted, Self::NotStarted) => true,
            (Self::Started, Self::Started) => true,
            (Self::Stopping, Self::Stopping) => true,
            (Self::Stopped, Self::Stopped) => true,
            (Self::StoppedWithError(_), Self::StoppedWithError(_)) => true,
            (_, _) => false,
        }
    }
}

#[derive(Debug)]
pub struct ServiceRunner<S>
where
    S: RunnableService,
{
    pub shared: Shared<S::SharedData>,
    state: Shared<watch::Sender<State>>,
}

impl<S> Clone for ServiceRunner<S>
where
    S: RunnableService,
{
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            state: self.state.clone(),
        }
    }
}

impl<S> ServiceRunner<S>
where
    S: RunnableService + 'static,
{
    pub fn new(service: S) -> Self {
        let shared = service.shared_data();
        let state = initialize_loop(service);
        Self { shared, state }
    }
}

#[async_trait::async_trait]
impl<S> Service for ServiceRunner<S>
where
    S: RunnableService + core::fmt::Debug,
    S::SharedData: core::fmt::Debug,
{
    fn start(&self) -> anyhow::Result<()> {
        let started = self.state.send_if_modified(|state| {
            if state.not_started() {
                *state = State::Started;
                true
            } else {
                false
            }
        });

        if started {
            Ok(())
        } else {
            Err(anyhow!("The service {:?} already has been started", self))
        }
    }

    fn stop(&self) -> bool {
        self.state.send_if_modified(|state| {
            if state.started() {
                *state = State::Stopping;
                true
            } else {
                false
            }
        })
    }

    async fn stop_and_await(&self) -> anyhow::Result<()> {
        let mut stop = self.state.subscribe();
        if stop.borrow().stopped() {
            Ok(())
        } else {
            self.stop();

            loop {
                if stop.borrow_and_update().stopped() {
                    return Ok(())
                }
                stop.changed().await?;
            }
        }
    }

    fn state(&self) -> State {
        self.state.borrow().clone()
    }
}

/// Initialize the background loop.
fn initialize_loop<S>(service: S) -> Shared<watch::Sender<State>>
where
    S: RunnableService + 'static,
{
    let (sender, receiver) = watch::channel(State::NotStarted);
    let state = Shared::new(sender);
    let stop_sender = state.clone();
    tokio::task::spawn(async move {
        let join_handler = run(service, receiver.clone());
        let result = join_handler.await;

        let stopped_state = if let Err(e) = result {
            State::StoppedWithError(e.to_string())
        } else {
            State::Stopped
        };

        let _ = stop_sender.send_if_modified(|state| {
            if !state.stopped() {
                *state = stopped_state;
                true
            } else {
                false
            }
        });
    });
    state
}

/// Main background run loop.
fn run<S>(mut service: S, mut state: watch::Receiver<State>) -> JoinHandle<()>
where
    S: RunnableService + 'static,
{
    tokio::task::spawn(async move {
        if state.borrow_and_update().not_started() {
            // We can panic here, because it is inside of the task.
            state.changed().await.expect("The service is destroyed");
        }

        // If the state after update is not `Started` then return to stop the service.
        if !state.borrow().started() {
            return
        }

        // We can panic here, because it is inside of the task.
        service
            .initialize()
            .await
            .expect("The initialization of the service field.");
        loop {
            tokio::select! {
                biased;

                _ = state.changed() => {
                    if state.borrow_and_update().started() {
                        return
                    }
                }

                result = service.run() => {
                    match result {
                        Ok(should_continue) => {
                            if !should_continue {
                                return
                            }
                        }
                        Err(e) => {
                            let e: &dyn std::error::Error = &*e;
                            tracing::error!(e);
                        }
                    }
                }
            }
        }
    })
}

// TODO: Add tests
