//! # State
//! Tracks all state that determines the actions of the relayer.

use core::ops::RangeInclusive;
pub use state_builder::*;
use std::ops::Deref;

mod state_builder;

#[cfg(test)]
mod test;

#[derive(Debug)]
/// The state of the Ethereum node.
pub struct EthState {
    /// The state that the relayer thinks the remote Ethereum node is in.
    remote: EthHeights,
    /// State related to the Ethereum node that is tracked by the relayer.
    local: Option<EthHeight>,
}

type EthHeight = u64;

#[derive(Clone, Debug)]
/// Type for tracking block height ranges.
struct Heights<T>(RangeInclusive<T>);

#[derive(Debug)]
/// Ethereum block height range.
struct EthHeights(Heights<u64>);

#[derive(Clone, Debug)]
/// The gap between the eth block height on
/// the relayer and the Ethereum node.
pub struct EthSyncGap(Heights<u64>);

#[derive(Clone, Debug)]
/// Block pagination to avoid requesting too
/// many logs within a single RPC call.
pub struct EthSyncPage {
    /// The range of this page.
    current: RangeInclusive<u64>,
    /// The size of the page.
    size: u64,
    /// The end of the pagination windows.
    end: u64,
}

impl EthState {
    /// Is the relayer in sync with the Ethereum node?
    pub fn is_synced(&self) -> bool {
        self.local
            .map_or(false, |local| local >= self.remote.finalized())
    }

    /// Get the gap between the relayer and the Ethereum node if
    /// a sync is required.
    pub fn needs_to_sync_eth(&self) -> Option<EthSyncGap> {
        (!self.is_synced()).then(|| {
            EthSyncGap::new(
                self.local.map(|l| l.saturating_add(1)).unwrap_or(0),
                self.remote.finalized(),
            )
        })
    }
}

impl EthHeights {
    /// Create a new Ethereum block height from the current
    /// block height and the desired finalization period.
    fn new(current: u64, finalization_period: u64) -> Self {
        Self(Heights(
            current.saturating_sub(finalization_period)..=current,
        ))
    }

    /// Get the finalized eth block height.
    fn finalized(&self) -> u64 {
        *self.0 .0.start()
    }
}

impl EthSyncGap {
    /// Create a new sync gap between the relayer and Ethereum node.
    pub(crate) fn new(local: u64, remote: u64) -> Self {
        Self(Heights(local..=remote))
    }

    /// Get the oldest block height (which will be the relayers eth block height).
    pub fn oldest(&self) -> u64 {
        *self.0 .0.start()
    }

    /// Get the latest block height (which will be the Ethereum nodes eth block height).
    pub fn latest(&self) -> u64 {
        *self.0 .0.end()
    }

    /// Create a pagination that will run from the oldest
    /// block to the latest. This will only request logs from
    /// up to the `page_size` number of blocks.
    pub fn page(&self, page_size: u64) -> Option<EthSyncPage> {
        let page = EthSyncPage {
            current: self.oldest()
                ..=self
                    .oldest()
                    .saturating_add(page_size.saturating_sub(1))
                    .min(self.latest()),
            size: page_size,
            end: self.latest(),
        };
        (!page.is_empty()).then_some(page)
    }
}

impl EthSyncPage {
    /// Reduce the pagination to the next page window or end.
    pub fn reduce(mut self) -> Option<Self> {
        self.current = self.current.start().saturating_add(self.size)
            ..=self.current.end().saturating_add(self.size).min(self.end);
        (!self.is_empty()).then_some(self)
    }

    /// Check if the pagination is empty (because the page size is zero
    /// or all the page windows have been consumed).
    pub fn is_empty(&self) -> bool {
        self.current.is_empty() || self.size == 0
    }

    /// Get the oldest block in this page window.
    pub fn oldest(&self) -> u64 {
        *self.current.start()
    }

    /// Get the latest block in this window.
    pub fn latest(&self) -> u64 {
        *self.current.end()
    }
}

impl Deref for EthHeights {
    type Target = Heights<u64>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
