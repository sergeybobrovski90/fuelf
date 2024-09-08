use crate::fuel_gas_price_updater::{
    DaGasPriceCommit,
    DaGasPriceSink,
    Error::CouldNotFetchDARecord,
    Result as GasPriceUpdaterResult,
};
use anyhow::anyhow;
use std::sync::Arc;
use tokio::sync::Mutex;

pub mod block_committer_source;
pub mod dummy_source;
pub mod service;

pub use block_committer_source::BlockCommitterDaGasPriceSource;
pub use dummy_source::DummyDaGasPriceSource;
pub use service::*;

pub const POLLING_INTERVAL_MS: u64 = 10_000;

pub type DaGasPriceProviderSink = Arc<Mutex<Option<DaGasPriceCommit>>>;

impl DaGasPriceSink for DaGasPriceProviderSink {
    fn get_da_commit(&mut self) -> GasPriceUpdaterResult<Option<DaGasPriceCommit>> {
        let mut metadata_guard = self.try_lock().map_err(|err| {
            CouldNotFetchDARecord(anyhow!(
                "Failed to lock shared da commit state: {:?}",
                err
            ))
        })?;

        let commit_details = metadata_guard.clone();

        // now mark it as consumed because we don't want to serve the same data
        // multiple times
        *metadata_guard = None;

        Ok(commit_details)
    }

    fn set_da_commit(
        &mut self,
        da_commit_details: DaGasPriceCommit,
    ) -> GasPriceUpdaterResult<()> {
        let mut metadata_guard = self.try_lock().map_err(|err| {
            CouldNotFetchDARecord(anyhow!(
                "Failed to lock shared metadata state: {:?}",
                err
            ))
        })?;

        *metadata_guard = Some(da_commit_details);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuel_gas_price_updater::{
        fuel_da_source_adapter::service::Result as DaGasPriceSourceResult,
        DaGasPriceProviderService,
        DaGasPriceSource,
        DaGasPriceSourceResponse,
        DummyDaGasPriceSource,
    };
    use fuel_core_services::{
        RunnableService,
        Service,
        ServiceRunner,
    };
    use std::time::Duration;
    use tokio::time::sleep;

    #[derive(Default)]
    struct FakeErroringMetadataIngestor;

    #[async_trait::async_trait]
    impl DaGasPriceSource for FakeErroringMetadataIngestor {
        async fn get_da_gas_price(
            &mut self,
        ) -> DaGasPriceSourceResult<DaGasPriceSourceResponse> {
            Err(anyhow!("boo!"))
        }
    }

    type TestValidService =
        DaGasPriceProviderService<DummyDaGasPriceSource, DaGasPriceProviderSink>;
    type TestErroringService =
        DaGasPriceProviderService<FakeErroringMetadataIngestor, DaGasPriceProviderSink>;

    #[tokio::test]
    async fn test_service_sets_cache_when_request_succeeds() {
        // given
        let service =
            TestValidService::new(DummyDaGasPriceSource, Some(Duration::from_millis(1)));

        let mut shared_state = service.shared_data();
        let service = ServiceRunner::new(service);

        // when
        service.start().unwrap();
        sleep(Duration::from_millis(10)).await;
        service.stop();

        // then
        let da_commit_details = shared_state.get_da_commit().unwrap();
        assert!(da_commit_details.is_some());
    }

    #[tokio::test]
    async fn test_service_invalidates_cache() {
        // given
        let service =
            TestValidService::new(DummyDaGasPriceSource, Some(Duration::from_millis(1)));
        let mut shared_state = service.shared_data();
        let service = ServiceRunner::new(service);

        // when
        service.start().unwrap();
        sleep(Duration::from_millis(10)).await;
        service.stop();
        let _ = shared_state.get_da_commit().unwrap();

        // then
        let da_commit_details = shared_state.get_da_commit().unwrap();
        assert!(da_commit_details.is_none());
    }

    #[tokio::test]
    async fn test_service_does_not_set_cache_when_request_fails() {
        // given
        let service = TestErroringService::new(
            FakeErroringMetadataIngestor,
            Some(Duration::from_millis(1)),
        );
        let mut shared_state = service.shared_data();
        let service = ServiceRunner::new(service);

        // when
        service.start().unwrap();
        sleep(Duration::from_millis(10)).await;
        service.stop();

        // then
        let da_commit_details = shared_state.get_da_commit().unwrap();
        assert!(da_commit_details.is_none());
    }
}
