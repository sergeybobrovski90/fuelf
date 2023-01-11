use std::sync::Arc;

use crate::{
    import::Params,
    ports,
    State,
};

use super::{
    import,
    sync,
};
use fuel_core_services::{
    KillSwitch,
    RunnableService,
    RunnableTask,
    SharedMutex,
    StateWatcher,
};
use fuel_core_types::blockchain::primitives::BlockHeight;
use futures::{
    stream::BoxStream,
    FutureExt,
};
use tokio::{
    sync::Notify,
    task::JoinHandle,
};

pub struct TaskSetup<P, E> {
    height_stream: BoxStream<'static, BlockHeight>,
    state: SharedMutex<State>,
    params: Params,
    p2p: Arc<P>,
    executor: Arc<E>,
}
pub struct Task {
    sync_task: Option<JoinHandle<()>>,
    import_task: Option<JoinHandle<()>>,
    kill_switch: KillSwitch,
}

#[async_trait::async_trait]
impl RunnableTask for Task {
    async fn run(
        &mut self,
        watcher: &mut fuel_core_services::StateWatcher,
    ) -> anyhow::Result<bool> {
        use futures::future::{
            select,
            Either,
        };
        if let (Some(sync_task), Some(import_task)) =
            (&mut self.sync_task, &mut self.import_task)
        {
            if let Either::Right((join_handles, _)) =
                select(watcher.changed().boxed(), select(sync_task, import_task)).await
            {
                match join_handles {
                    Either::Left((_, import_task)) => {
                        self.kill_switch.kill_all();
                        let _ = import_task.await;
                        return Ok(false)
                    }
                    Either::Right((import_result, _)) => {
                        return Ok(import_result.map(|_| true)?)
                    }
                }
            }
        }
        if !watcher.borrow().started() {
            self.kill_switch.kill_all();
            if let Some(sync_task) = self.sync_task.take() {
                let _ = sync_task.await;
            }
            if let Some(import_task) = self.import_task.take() {
                let _ = import_task.await;
            }
            Ok(false)
        } else {
            Ok(true)
        }
    }
}

#[async_trait::async_trait]
impl<P, E> RunnableService for TaskSetup<P, E>
where
    P: ports::PeerToPeer + Send + Sync + 'static,
    E: ports::Executor + Send + Sync + 'static,
{
    const NAME: &'static str = "fuel-core-sync";

    type SharedData = ();

    type Task = Task;

    fn shared_data(&self) -> Self::SharedData {}

    async fn into_task(self, _: &StateWatcher) -> anyhow::Result<Self::Task> {
        let TaskSetup {
            height_stream,
            state,
            params,
            p2p,
            executor,
        } = self;
        let kill_switch = KillSwitch::new();
        let notify = Arc::new(Notify::new());
        let sync_task = sync::spawn_sync(
            height_stream,
            state.clone(),
            notify.clone(),
            kill_switch.handle(),
        );
        let import_task = tokio::spawn(import::import(
            state,
            notify,
            params,
            p2p,
            executor,
            kill_switch.handle(),
        ));
        Ok(Task {
            sync_task: Some(sync_task),
            import_task: Some(import_task),
            kill_switch,
        })
    }
}
