#![deny(clippy::arithmetic_side_effects)]
#![deny(clippy::cast_possible_truncation)]
#![deny(unused_crate_dependencies)]
#![deny(warnings)]

use async_trait::async_trait;
use fuel_core_services::{
    RunnableService,
    RunnableTask,
    StateWatcher,
};
use fuel_core_types::fuel_types::BlockHeight;
use std::sync::Arc;

use tokio::sync::RwLock;

pub mod static_updater;

pub mod fuel_gas_price_updater;

// pub fn new_service<A, U>(
//     current_fuel_block_height: BlockHeight,
//     update_algo: U,
// ) -> anyhow::Result<ServiceRunner<GasPriceService<A, U>>>
// where
//     U: UpdateAlgorithm<Algorithm = A> + Send + Sync,
//     A: Send + Sync,
// {
//     let shared_algo
//     let service = GasPriceService::new(current_fuel_block_height, update_algo, SharedGasPriceAlgo::new(update_algo.start(current_fuel_block_height));
//     Ok(ServiceRunner::new(service))
// }

/// The service that updates the gas price algorithm.
pub struct GasPriceService<A, U> {
    /// The algorithm that can be used in the next block
    next_block_algorithm: SharedGasPriceAlgo<A>,
    /// The code that is run to update your specific algorithm
    update_algorithm: U,
}

impl<A, U> GasPriceService<A, U>
where
    U: UpdateAlgorithm<Algorithm = A>,
    A: Send + Sync,
{
    pub async fn new(
        starting_block_height: BlockHeight,
        update_algorithm: U,
        mut shared_algo: SharedGasPriceAlgo<A>,
    ) -> Self {
        let algorithm = update_algorithm.start(starting_block_height);
        shared_algo.update(algorithm).await;
        Self {
            next_block_algorithm: shared_algo,
            update_algorithm,
        }
    }

    pub fn next_block_algorithm(&self) -> SharedGasPriceAlgo<A> {
        self.next_block_algorithm.clone()
    }
}

/// The interface the Service has with the code that updates the gas price algorithm.
#[async_trait]
pub trait UpdateAlgorithm {
    /// The type of the algorithm that is being updated
    type Algorithm;

    /// Start the algorithm at a given block height
    fn start(&self, for_block: BlockHeight) -> Self::Algorithm;

    /// Wait for the next algorithm to be available
    async fn next(&mut self) -> anyhow::Result<Self::Algorithm>;
}

pub trait GasPriceAlgorithm {
    fn next_gas_price(&self) -> u64;
    fn worst_case_gas_price(&self, block_height: BlockHeight) -> u64;
}

impl<A, U> GasPriceService<A, U>
where
    U: UpdateAlgorithm<Algorithm = A>,
    A: Send + Sync,
{
    async fn update(&mut self, new_algorithm: A) {
        self.next_block_algorithm.update(new_algorithm).await;
    }
}

#[derive(Debug, Default)]
pub struct SharedGasPriceAlgo<A>(Arc<RwLock<Option<A>>>);

impl<A> Clone for SharedGasPriceAlgo<A> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<A> SharedGasPriceAlgo<A>
where
    A: Send + Sync,
{
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(None)))
    }

    pub fn new_with_algorithm(algorithm: A) -> Self {
        Self(Arc::new(RwLock::new(Some(algorithm))))
    }

    pub async fn update(&mut self, new_algo: A) {
        let mut write_lock = self.0.write().await;
        *write_lock = Some(new_algo);
    }
}

impl<A> SharedGasPriceAlgo<A>
where
    A: GasPriceAlgorithm + Send + Sync,
{
    pub async fn next_gas_price(&self) -> Option<u64> {
        self.0.read().await.as_ref().map(|a| a.next_gas_price())
    }

    pub async fn worst_case_gas_price(&self, block_height: BlockHeight) -> Option<u64> {
        self.0
            .read()
            .await
            .as_ref()
            .map(|a| a.worst_case_gas_price(block_height))
    }
}

#[async_trait]
impl<A, U> RunnableService for GasPriceService<A, U>
where
    U: UpdateAlgorithm<Algorithm = A> + Send + Sync,
    A: Send + Sync,
{
    const NAME: &'static str = "GasPriceUpdater";
    type SharedData = SharedGasPriceAlgo<A>;
    type Task = Self;
    type TaskParams = ();

    fn shared_data(&self) -> Self::SharedData {
        self.next_block_algorithm.clone()
    }

    async fn into_task(
        self,
        _state_watcher: &StateWatcher,
        _params: Self::TaskParams,
    ) -> anyhow::Result<Self::Task> {
        Ok(self)
    }
}

#[async_trait]
impl<A, U> RunnableTask for GasPriceService<A, U>
where
    U: UpdateAlgorithm<Algorithm = A> + Send + Sync,
    A: Send + Sync,
{
    async fn run(&mut self, watcher: &mut StateWatcher) -> anyhow::Result<bool> {
        let should_continue;
        tokio::select! {
            biased;
            _ = watcher.while_started() => {
                tracing::debug!("Stopping gas price service");
                should_continue = false;
            }
            new_algo = self.update_algorithm.next() => {
                let new_algo = new_algo?;
                tracing::debug!("Updating gas price algorithm");
                self.update(new_algo).await;
                should_continue = true;
            }
        }
        Ok(should_continue)
    }

    async fn shutdown(self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[allow(clippy::arithmetic_side_effects)]
#[allow(non_snake_case)]
#[cfg(test)]
mod tests {
    use crate::{
        GasPriceAlgorithm,
        GasPriceService,
        SharedGasPriceAlgo,
        UpdateAlgorithm,
    };
    use fuel_core_services::{
        RunnableService,
        Service,
        ServiceRunner,
        StateWatcher,
    };
    use fuel_core_types::fuel_types::BlockHeight;
    use tokio::sync::mpsc;

    #[derive(Clone, Debug)]
    struct TestAlgorithm {
        price: u64,
    }

    impl GasPriceAlgorithm for TestAlgorithm {
        fn next_gas_price(&self) -> u64 {
            self.price
        }

        fn worst_case_gas_price(&self, _block_height: BlockHeight) -> u64 {
            self.price
        }
    }

    struct TestAlgorithmUpdater {
        start: TestAlgorithm,
        price_source: mpsc::Receiver<u64>,
    }

    #[async_trait::async_trait]
    impl UpdateAlgorithm for TestAlgorithmUpdater {
        type Algorithm = TestAlgorithm;

        fn start(&self, _for_block: BlockHeight) -> Self::Algorithm {
            self.start.clone()
        }

        async fn next(&mut self) -> anyhow::Result<Self::Algorithm> {
            let price = self.price_source.recv().await.unwrap();
            Ok(TestAlgorithm { price })
        }
    }
    #[tokio::test]
    async fn run__updates_gas_price() {
        // given
        let (price_sender, price_receiver) = mpsc::channel(1);
        let updater = TestAlgorithmUpdater {
            start: TestAlgorithm { price: 0 },
            price_source: price_receiver,
        };
        let shared_algo = SharedGasPriceAlgo::new();
        let service = GasPriceService::new(0.into(), updater, shared_algo).await;
        let watcher = StateWatcher::started();
        let read_algo = service.next_block_algorithm();
        let task = service.into_task(&watcher, ()).await.unwrap();
        let expected_price = 100;
        let service = ServiceRunner::new(task);
        service.start().unwrap();

        // when
        price_sender.send(expected_price).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // then
        let actual_price = read_algo.next_gas_price().await.unwrap();
        assert_eq!(expected_price, actual_price);
    }
}
