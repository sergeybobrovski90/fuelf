use crate::schema::tx::types::TransactionStatus;
use fuel_core_interfaces::common::prelude::Bytes32;
use futures::{
    stream::BoxStream,
    Stream,
    StreamExt,
    TryStreamExt,
};
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

#[cfg(test)]
mod test;

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub(crate) trait TxnStatusChangeState {
    /// Return the transaction status from the tx pool and database.
    async fn get_tx_status(
        &self,
        id: fuel_core_interfaces::common::fuel_types::Bytes32,
    ) -> anyhow::Result<Option<TransactionStatus>>;
}

pub(crate) async fn transaction_status_change(
    state: Box<dyn TxnStatusChangeState + Send + Sync>,
    stream: BoxStream<'static, Result<Bytes32, BroadcastStreamRecvError>>,
    transaction_id: Bytes32,
) -> impl Stream<Item = anyhow::Result<TransactionStatus>> {
    let transaction_id = transaction_id.into();

    let check_db_first = state.get_tx_status(transaction_id).await.transpose();
    let (close, mut closed) = tokio::sync::oneshot::channel();
    let mut close = Some(close);

    let stream = futures::stream::unfold((state, stream), move |(state, mut stream)| {
        let is_closed = !matches!(
            closed.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        );
        async move {
            if is_closed {
                return None
            }
            match stream.next().await {
                // Got status update that matches the transaction_id we are looking for.
                Some(Ok(id)) if id == transaction_id.into() => {
                    let status = state.get_tx_status(transaction_id).await;
                    match status {
                        // Got the status from the db.
                        Ok(Some(s)) => Some((Some(Ok(s)), (state, stream))),
                        // Could not get status from the db so the stream must exit
                        // as a value has been missed and the only valid thing to do
                        // is to restart the stream.
                        Ok(None) => None,
                        // Got an error so return it.
                        Err(e) => Some((Some(Err(e)), (state, stream))),
                    }
                }
                // Got a status update but it's not this transaction so ignore it.
                Some(Ok(_)) => Some((None, (state, stream))),
                // Buffer filled up before this stream was polled.
                Some(Err(BroadcastStreamRecvError::Lagged(_))) => {
                    // Check the db incase a missed status was our transaction.
                    let status = state.get_tx_status(transaction_id).await.transpose();
                    Some((status, (state, stream)))
                }
                // Channel is closed.
                None => None,
            }
        }
    });

    // CHeck the database first incase there is already a status.
    futures::stream::once(futures::future::ready(check_db_first))
            // Then wait for a status update.
            .chain(stream)
            // Filter out values that don't apply to this query.
            .filter_map(futures::future::ready)
            // Continue this stream while a submitted status is received
            // as this stream should only end when no more status updates are possible.
            .map_ok(move |status| {
                if !matches!(status, TransactionStatus::Submitted(_)) {
                    if let Some(close) = close.take() {
                        let _ = close.send(());
                    }
                }
                status
            })
}
