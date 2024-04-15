use crate::config::Config;
use fuel_core_executor::{
    executor::{
        ExecutionBlockWithSource,
        ExecutionOptions,
        OnceTransactionsSource,
    },
    ports::{
        RelayerPort,
        TransactionsSource,
    },
};
use fuel_core_storage::{
    column::Column,
    kv_store::KeyValueInspect,
    transactional::{
        AtomicView,
        Changes,
        Modifiable,
    },
};
use fuel_core_types::{
    blockchain::primitives::DaBlockHeight,
    fuel_tx::Transaction,
    fuel_types::BlockHeight,
    services::{
        block_producer::Components,
        executor::{
            Error as ExecutorError,
            ExecutionResult,
            ExecutionTypes,
            Result as ExecutorResult,
            TransactionExecutionStatus,
        },
        Uncommitted,
    },
};
use std::sync::Arc;

#[cfg(feature = "wasm-executor")]
use fuel_core_storage::{
    not_found,
    structured_storage::StructuredStorage,
    tables::{
        StateTransitionBytecodeVersions,
        UploadedBytecodes,
    },
    StorageAsRef,
};
use fuel_core_types::blockchain::header::StateTransitionBytecodeVersion;
#[cfg(any(test, feature = "test-helpers"))]
use fuel_core_types::services::executor::UncommittedResult;

#[cfg(feature = "wasm-executor")]
enum ExecutionStrategy {
    /// The native executor used when the version matches.
    Native,
    /// The WASM executor used even when the version matches.
    Wasm {
        /// The compiled WASM module of the native executor bytecode.
        module: wasmtime::Module,
    },
}

/// The upgradable executor supports the WASM version of the state transition function.
/// If the block has a version the same as a native executor, we will use it.
/// If not, the WASM version of the state transition function will be used
/// (if the database has a corresponding bytecode).
pub struct Executor<S, R> {
    pub storage_view_provider: S,
    pub relayer_view_provider: R,
    pub config: Arc<Config>,
    #[cfg(feature = "wasm-executor")]
    engine: wasmtime::Engine,
    #[cfg(feature = "wasm-executor")]
    execution_strategy: ExecutionStrategy,
    #[cfg(feature = "wasm-executor")]
    cached_modules: parking_lot::Mutex<
        std::collections::HashMap<
            fuel_core_types::blockchain::header::StateTransitionBytecodeVersion,
            wasmtime::Module,
        >,
    >,
}

#[cfg(feature = "wasm-executor")]
mod private {
    use std::sync::OnceLock;
    use wasmtime::{
        Engine,
        Module,
    };

    /// The default engine for the WASM executor. It is used to compile the WASM bytecode.
    pub(crate) static DEFAULT_ENGINE: OnceLock<Engine> = OnceLock::new();

    /// The default module compiles the WASM bytecode of the native executor.
    /// It is used to create the WASM instance of the executor.
    pub(crate) static COMPILED_UNDERLYING_EXECUTOR: OnceLock<Module> = OnceLock::new();
}

#[cfg(feature = "wasm-executor")]
/// The environment variable that forces the executor to use the WASM
/// version of the state transition function.
pub const FUEL_ALWAYS_USE_WASM: &str = "FUEL_ALWAYS_USE_WASM";

impl<S, R> Executor<S, R> {
    /// The current version of the native executor is used to determine whether
    /// we need to use a native executor or WASM. If the version is the same as
    /// on the block, native execution is used. If the version is not the same
    /// as in the block, then the WASM executor is used.
    pub const VERSION: u32 = 0;
    /// This constant is used along with the `version_check` test.
    /// To avoid automatic bumping during release, the constant uses `-` instead of `.`.
    #[cfg(test)]
    pub const CRATE_VERSION: &'static str = "0-24-2";

    pub fn new(
        storage_view_provider: S,
        relayer_view_provider: R,
        config: Config,
    ) -> Self {
        #[cfg(feature = "wasm-executor")]
        {
            if std::env::var_os(FUEL_ALWAYS_USE_WASM).is_some() {
                Self::wasm(storage_view_provider, relayer_view_provider, config)
            } else {
                Self::native(storage_view_provider, relayer_view_provider, config)
            }
        }
        #[cfg(not(feature = "wasm-executor"))]
        {
            Self::native(storage_view_provider, relayer_view_provider, config)
        }
    }

    pub fn native_executor_version(&self) -> StateTransitionBytecodeVersion {
        self.config.native_executor_version.unwrap_or(Self::VERSION)
    }

    pub fn native(
        storage_view_provider: S,
        relayer_view_provider: R,
        config: Config,
    ) -> Self {
        Self {
            storage_view_provider,
            relayer_view_provider,
            config: Arc::new(config),
            #[cfg(feature = "wasm-executor")]
            engine: private::DEFAULT_ENGINE
                .get_or_init(wasmtime::Engine::default)
                .clone(),
            #[cfg(feature = "wasm-executor")]
            execution_strategy: ExecutionStrategy::Native,
            #[cfg(feature = "wasm-executor")]
            cached_modules: Default::default(),
        }
    }

    #[cfg(feature = "wasm-executor")]
    pub fn wasm(
        storage_view_provider: S,
        relayer_view_provider: R,
        config: Config,
    ) -> Self {
        let engine = private::DEFAULT_ENGINE.get_or_init(wasmtime::Engine::default);
        let module = private::COMPILED_UNDERLYING_EXECUTOR.get_or_init(|| {
            wasmtime::Module::new(engine, crate::WASM_BYTECODE)
                .expect("Failed to compile the WASM bytecode")
        });

        Self {
            storage_view_provider,
            relayer_view_provider,
            config: Arc::new(config),
            engine: engine.clone(),
            execution_strategy: ExecutionStrategy::Wasm {
                module: module.clone(),
            },
            cached_modules: Default::default(),
        }
    }
}

impl<D, R> Executor<D, R>
where
    R: AtomicView<Height = DaBlockHeight>,
    R::View: RelayerPort + Send + Sync + 'static,
    D: AtomicView<Height = BlockHeight> + Modifiable,
    D::View: KeyValueInspect<Column = Column> + Send + Sync + 'static,
{
    #[cfg(any(test, feature = "test-helpers"))]
    /// Executes the block and commits the result of the execution into the inner `Database`.
    pub fn execute_and_commit(
        &mut self,
        block: fuel_core_types::services::executor::ExecutionBlock,
    ) -> fuel_core_types::services::executor::Result<ExecutionResult> {
        let (result, changes) = self.execute_without_commit(block)?.into();

        self.storage_view_provider.commit_changes(changes)?;
        Ok(result)
    }
}

impl<S, R> Executor<S, R>
where
    S: AtomicView<Height = BlockHeight>,
    S::View: KeyValueInspect<Column = Column> + Send + Sync + 'static,
    R: AtomicView<Height = DaBlockHeight>,
    R::View: RelayerPort + Send + Sync + 'static,
{
    #[cfg(any(test, feature = "test-helpers"))]
    /// Executes the block and returns the result of the execution with storage changes.
    pub fn execute_without_commit(
        &self,
        block: fuel_core_types::services::executor::ExecutionBlock,
    ) -> fuel_core_types::services::executor::Result<UncommittedResult<Changes>> {
        self.execute_without_commit_with_coinbase(block, Default::default(), 0)
    }

    #[cfg(any(test, feature = "test-helpers"))]
    /// The analog of the [`Self::execute_without_commit`] method,
    /// but with the ability to specify the coinbase recipient and the gas price.
    pub fn execute_without_commit_with_coinbase(
        &self,
        block: fuel_core_types::services::executor::ExecutionBlock,
        coinbase_recipient: fuel_core_types::fuel_types::ContractId,
        gas_price: u64,
    ) -> fuel_core_types::services::executor::Result<UncommittedResult<Changes>> {
        let component = match block {
            ExecutionTypes::DryRun(_) => {
                panic!("It is not possible to commit the dry run result");
            }
            ExecutionTypes::Production(block) => ExecutionTypes::Production(Components {
                header_to_produce: block.header,
                transactions_source: OnceTransactionsSource::new(block.transactions),
                coinbase_recipient,
                gas_price,
            }),
            ExecutionTypes::Validation(block) => ExecutionTypes::Validation(block),
        };

        let option = self.config.as_ref().into();
        self.execute_inner(component, option)
    }
}

impl<S, R> Executor<S, R>
where
    S: AtomicView,
    S::View: KeyValueInspect<Column = Column> + Send + Sync + 'static,
    R: AtomicView<Height = DaBlockHeight>,
    R::View: RelayerPort + Send + Sync + 'static,
{
    /// Executes the block and returns the result of the execution without committing the changes.
    pub fn execute_without_commit_with_source<TxSource>(
        &self,
        block: ExecutionBlockWithSource<TxSource>,
    ) -> ExecutorResult<Uncommitted<ExecutionResult, Changes>>
    where
        TxSource: TransactionsSource + Send + Sync + 'static,
    {
        let options = self.config.as_ref().into();
        self.execute_inner(block, options)
    }

    /// Executes the block and returns the result of the execution without committing
    /// the changes in the dry run mode.
    pub fn dry_run(
        &self,
        component: Components<Vec<Transaction>>,
        utxo_validation: Option<bool>,
    ) -> ExecutorResult<Vec<TransactionExecutionStatus>> {
        // fallback to service config value if no utxo_validation override is provided
        let utxo_validation =
            utxo_validation.unwrap_or(self.config.utxo_validation_default);

        let options = ExecutionOptions {
            utxo_validation,
            backtrace: self.config.backtrace,
        };

        let component = Components {
            header_to_produce: component.header_to_produce,
            transactions_source: OnceTransactionsSource::new(
                component.transactions_source,
            ),
            coinbase_recipient: Default::default(),
            gas_price: component.gas_price,
        };

        let ExecutionResult {
            skipped_transactions,
            tx_status,
            ..
        } = self
            .execute_inner(ExecutionTypes::DryRun(component), options)?
            .into_result();

        // If one of the transactions fails, return an error.
        if let Some((_, err)) = skipped_transactions.into_iter().next() {
            return Err(err)
        }

        Ok(tx_status)
    }

    #[cfg(feature = "wasm-executor")]
    fn execute_inner<TxSource>(
        &self,
        block: ExecutionBlockWithSource<TxSource>,
        options: ExecutionOptions,
    ) -> ExecutorResult<Uncommitted<ExecutionResult, Changes>>
    where
        TxSource: TransactionsSource + Send + Sync + 'static,
    {
        let block_version = block.state_transition_version();
        let native_executor_version = self.native_executor_version();
        if block_version == native_executor_version {
            match &self.execution_strategy {
                ExecutionStrategy::Native => self.native_execute_inner(block, options),
                ExecutionStrategy::Wasm { module } => {
                    self.wasm_execute_inner(module, block, options)
                }
            }
        } else {
            let module = self.get_module(block_version)?;
            tracing::warn!(
                "The block version({}) is different from the native executor version({}). \
                The WASM executor will be used.", block_version, Self::VERSION
            );
            self.wasm_execute_inner(&module, block, options)
        }
    }

    #[cfg(not(feature = "wasm-executor"))]
    fn execute_inner<TxSource>(
        &self,
        block: ExecutionBlockWithSource<TxSource>,
        options: ExecutionOptions,
    ) -> ExecutorResult<Uncommitted<ExecutionResult, Changes>>
    where
        TxSource: TransactionsSource + Send + Sync + 'static,
    {
        let block_version = block.state_transition_version();
        let native_executor_version = self.native_executor_version();
        if block_version == native_executor_version {
            self.native_execute_inner(block, options)
        } else {
            Err(ExecutorError::Other(format!(
                "Not supported version `{block_version}`. Expected version is `{}`",
                Self::VERSION
            )))
        }
    }

    #[cfg(feature = "wasm-executor")]
    fn wasm_execute_inner<TxSource>(
        &self,
        module: &wasmtime::Module,
        block: ExecutionBlockWithSource<TxSource>,
        options: ExecutionOptions,
    ) -> ExecutorResult<Uncommitted<ExecutionResult, Changes>>
    where
        TxSource: TransactionsSource + Send + Sync + 'static,
    {
        let mut source = None;
        let block = block.map_p(|component| {
            let Components {
                header_to_produce,
                transactions_source,
                coinbase_recipient,
                gas_price,
            } = component;

            source = Some(transactions_source);

            Components {
                header_to_produce,
                transactions_source: (),
                coinbase_recipient,
                gas_price,
            }
        });

        let storage = self.storage_view_provider.latest_view();
        let relayer = self.relayer_view_provider.latest_view();

        let instance = crate::instance::Instance::new(&self.engine)
            .add_source(source)?
            .add_storage(storage)?
            .add_relayer(relayer)?
            .add_input_data(block, options)?;

        instance.run(module)
    }

    fn native_execute_inner<TxSource>(
        &self,
        block: ExecutionBlockWithSource<TxSource>,
        options: ExecutionOptions,
    ) -> ExecutorResult<Uncommitted<ExecutionResult, Changes>>
    where
        TxSource: TransactionsSource + Send + Sync + 'static,
    {
        let storage = self.storage_view_provider.latest_view();
        let relayer = self.relayer_view_provider.latest_view();

        let instance = fuel_core_executor::executor::ExecutionInstance {
            relayer,
            database: storage,
            options,
        };
        instance.execute_without_commit(block)
    }

    /// Returns the compiled WASM module of the state transition function.
    ///
    /// Note: The method compiles the WASM module if it is not cached.
    ///     It is a long process to call this method, which can block the thread.
    #[cfg(feature = "wasm-executor")]
    fn get_module(
        &self,
        version: fuel_core_types::blockchain::header::StateTransitionBytecodeVersion,
    ) -> ExecutorResult<wasmtime::Module> {
        let guard = self.cached_modules.lock();
        if let Some(module) = guard.get(&version) {
            return Ok(module.clone());
        }
        drop(guard);

        let view = StructuredStorage::new(self.storage_view_provider.latest_view());
        let bytecode_root = *view
            .storage::<StateTransitionBytecodeVersions>()
            .get(&version)?
            .ok_or(not_found!(StateTransitionBytecodeVersions))?;
        let uploaded_bytecode = view
            .storage::<UploadedBytecodes>()
            .get(&bytecode_root)?
            .ok_or(not_found!(UploadedBytecodes))?;

        let fuel_core_types::fuel_vm::UploadedBytecode::Completed(bytecode) =
            uploaded_bytecode.as_ref()
        else {
            return Err(ExecutorError::Other(format!(
                "The bytecode under the bytecode_root(`{bytecode_root}`) is not completed",
            )))
        };

        // Compiles the module
        let module = wasmtime::Module::new(&self.engine, bytecode).map_err(|e| {
            ExecutorError::Other(format!(
                "Failed to compile the module for the version `{}` with {e}",
                version,
            ))
        })?;

        self.cached_modules.lock().insert(version, module.clone());
        Ok(module)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use fuel_core_storage::{
        kv_store::Value,
        structured_storage::test::InMemoryStorage,
        tables::ConsensusParametersVersions,
        transactional::WriteTransaction,
        Result as StorageResult,
        StorageAsMut,
    };
    use fuel_core_types::{
        blockchain::{
            block::{
                Block,
                PartialFuelBlock,
            },
            header::{
                ApplicationHeader,
                ConsensusHeader,
                PartialBlockHeader,
                StateTransitionBytecodeVersion,
            },
            primitives::Empty,
        },
        fuel_tx::{
            AssetId,
            Bytes32,
        },
        services::relayer::Event,
        tai64::Tai64,
    };

    #[derive(Clone, Debug)]
    struct Storage(InMemoryStorage<Column>);

    impl AtomicView for Storage {
        type View = InMemoryStorage<Column>;
        type Height = BlockHeight;

        fn latest_height(&self) -> Option<Self::Height> {
            None
        }

        fn view_at(&self, _: &Self::Height) -> StorageResult<Self::View> {
            unimplemented!()
        }

        fn latest_view(&self) -> Self::View {
            self.0.clone()
        }
    }

    impl KeyValueInspect for Storage {
        type Column = Column;

        fn get(&self, key: &[u8], column: Self::Column) -> StorageResult<Option<Value>> {
            self.0.get(key, column)
        }
    }

    impl Modifiable for Storage {
        fn commit_changes(&mut self, changes: Changes) -> StorageResult<()> {
            self.0.commit_changes(changes)
        }
    }

    #[derive(Copy, Clone)]
    struct DisabledRelayer;

    impl RelayerPort for DisabledRelayer {
        fn enabled(&self) -> bool {
            false
        }

        fn get_events(&self, _: &DaBlockHeight) -> anyhow::Result<Vec<Event>> {
            unimplemented!()
        }
    }

    impl AtomicView for DisabledRelayer {
        type View = Self;
        type Height = DaBlockHeight;

        fn latest_height(&self) -> Option<Self::Height> {
            None
        }

        fn view_at(&self, _: &Self::Height) -> StorageResult<Self::View> {
            unimplemented!()
        }

        fn latest_view(&self) -> Self::View {
            *self
        }
    }

    #[test]
    fn version_check() {
        let crate_version = env!("CARGO_PKG_VERSION");
        let executor_cate_version = Executor::<Storage, DisabledRelayer>::CRATE_VERSION
            .to_string()
            .replace('-', ".");
        assert_eq!(
            executor_cate_version, crate_version,
            "When this test fails, \
            it is a sign that maybe we need to increase the `Executor::VERSION`. \
            If there are no breaking changes that affect the execution, \
            then you can only increase `Executor::CRATE_VERSION` to pass this test."
        );
    }

    const CONSENSUS_PARAMETERS_VERSION: u32 = 0;

    fn storage() -> Storage {
        let mut storage = Storage(InMemoryStorage::default());
        let mut tx = storage.write_transaction();
        tx.storage_as_mut::<ConsensusParametersVersions>()
            .insert(&CONSENSUS_PARAMETERS_VERSION, &Default::default())
            .unwrap();
        tx.commit().unwrap();

        storage
    }

    fn valid_block(
        state_transition_bytecode_version: StateTransitionBytecodeVersion,
    ) -> Block {
        PartialFuelBlock::new(
            PartialBlockHeader {
                application: ApplicationHeader {
                    da_height: Default::default(),
                    consensus_parameters_version: CONSENSUS_PARAMETERS_VERSION,
                    state_transition_bytecode_version,
                    generated: Empty,
                },
                consensus: ConsensusHeader {
                    prev_root: Default::default(),
                    height: Default::default(),
                    time: Tai64::now(),
                    generated: Empty,
                },
            },
            vec![Transaction::mint(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                AssetId::BASE,
                Default::default(),
            )
            .into()],
        )
        .generate(&[], Bytes32::zeroed())
    }

    #[cfg(not(feature = "wasm-executor"))]
    mod native {
        use super::*;
        use crate::executor::Executor;

        #[test]
        fn can_validate_block() {
            let storage = storage();
            let executor = Executor::native(storage, DisabledRelayer, Config::default());

            // Given
            let block = valid_block(Executor::<Storage, DisabledRelayer>::VERSION);

            // When
            let result = executor
                .execute_without_commit(ExecutionTypes::Validation(block))
                .map(|_| ());

            // Then
            assert_eq!(Ok(()), result);
        }

        #[test]
        fn validation_fails_because_of_versions_mismatch() {
            let storage = storage();
            let executor = Executor::native(storage, DisabledRelayer, Config::default());

            // Given
            let wrong_version = Executor::<Storage, DisabledRelayer>::VERSION + 1;
            let block = valid_block(wrong_version);

            // When
            let result = executor
                .execute_without_commit(ExecutionTypes::Validation(block))
                .map(|_| ());

            // Then
            result.expect_err("The validation should fail because of versions mismatch");
        }
    }

    #[cfg(feature = "wasm-executor")]
    #[allow(non_snake_case)]
    mod wasm {
        use super::*;
        use crate::{
            executor::Executor,
            WASM_BYTECODE,
        };
        use fuel_core_storage::tables::UploadedBytecodes;
        use fuel_core_types::fuel_vm::UploadedBytecode;

        #[test]
        fn can_validate_block__native_strategy() {
            let storage = storage();

            // Given
            let executor = Executor::native(storage, DisabledRelayer, Config::default());
            let block = valid_block(Executor::<Storage, DisabledRelayer>::VERSION);

            // When
            let result = executor
                .execute_without_commit(ExecutionTypes::Validation(block))
                .map(|_| ());

            // Then
            assert_eq!(Ok(()), result);
        }

        #[test]
        fn can_validate_block__wasm_strategy() {
            let storage = storage();

            // Given
            let executor = Executor::wasm(storage, DisabledRelayer, Config::default());
            let block = valid_block(Executor::<Storage, DisabledRelayer>::VERSION);

            // When
            let result = executor
                .execute_without_commit(ExecutionTypes::Validation(block))
                .map(|_| ());

            // Then
            assert_eq!(Ok(()), result);
        }

        #[test]
        fn validation_fails_because_of_versions_mismatch__native_strategy() {
            let storage = storage();

            // Given
            let executor = Executor::native(storage, DisabledRelayer, Config::default());
            let wrong_version = Executor::<Storage, DisabledRelayer>::VERSION + 1;
            let block = valid_block(wrong_version);

            // When
            let result = executor
                .execute_without_commit(ExecutionTypes::Validation(block))
                .map(|_| ());

            // Then
            result.expect_err("The validation should fail because of versions mismatch");
        }

        #[test]
        fn validation_fails_because_of_versions_mismatch__wasm_strategy() {
            let storage = storage();

            // Given
            let executor = Executor::wasm(storage, DisabledRelayer, Config::default());
            let wrong_version = Executor::<Storage, DisabledRelayer>::VERSION + 1;
            let block = valid_block(wrong_version);

            // When
            let result = executor
                .execute_without_commit(ExecutionTypes::Validation(block))
                .map(|_| ());

            // Then
            result.expect_err("The validation should fail because of versions mismatch");
        }

        fn storage_with_state_transition(
            next_version: StateTransitionBytecodeVersion,
        ) -> Storage {
            // Only FuelVM requires the Merkle root to match the corresponding bytecode
            // during uploading of it. The executor itself only uses a database to get the code,
            // and how bytecode appeared there is not the executor's responsibility.
            const BYTECODE_ROOT: Bytes32 = Bytes32::zeroed();

            let mut storage = storage();
            let mut tx = storage.write_transaction();
            tx.storage_as_mut::<StateTransitionBytecodeVersions>()
                .insert(&next_version, &BYTECODE_ROOT)
                .unwrap();
            tx.storage_as_mut::<UploadedBytecodes>()
                .insert(
                    &BYTECODE_ROOT,
                    &UploadedBytecode::Completed(WASM_BYTECODE.to_vec()),
                )
                .unwrap();
            tx.commit().unwrap();

            storage
        }

        #[test]
        fn can_validate_block_with_next_version__native_strategy() {
            // Given
            let next_version = Executor::<Storage, DisabledRelayer>::VERSION + 1;
            let storage = storage_with_state_transition(next_version);
            let executor = Executor::native(storage, DisabledRelayer, Config::default());
            let block = valid_block(next_version);

            // When
            let result = executor
                .execute_without_commit(ExecutionTypes::Validation(block))
                .map(|_| ());

            // Then
            assert_eq!(Ok(()), result);
        }

        #[test]
        fn can_validate_block_with_next_version__wasm_strategy() {
            // Given
            let next_version = Executor::<Storage, DisabledRelayer>::VERSION + 1;
            let storage = storage_with_state_transition(next_version);
            let executor = Executor::wasm(storage, DisabledRelayer, Config::default());
            let block = valid_block(next_version);

            // When
            let result = executor
                .execute_without_commit(ExecutionTypes::Validation(block))
                .map(|_| ());

            // Then
            assert_eq!(Ok(()), result);
        }

        // The test verifies that `Executor::get_module` method caches the compiled WASM module.
        // If it doesn't cache the modules, the test will fail with a timeout.
        #[test]
        #[ntest::timeout(60_000)]
        fn reuse_cached_compiled_module__native_strategy() {
            // Given
            let next_version = Executor::<Storage, DisabledRelayer>::VERSION + 1;
            let storage = storage_with_state_transition(next_version);
            let executor = Executor::native(storage, DisabledRelayer, Config::default());
            let block = valid_block(next_version);

            // When
            for _ in 0..1000 {
                let result = executor
                    .execute_without_commit(ExecutionTypes::Validation(block.clone()))
                    .map(|_| ());

                // Then
                assert_eq!(Ok(()), result);
            }
        }

        // The test verifies that `Executor::get_module` method caches the compiled WASM module.
        // If it doesn't cache the modules, the test will fail with a timeout.
        #[test]
        #[ntest::timeout(60_000)]
        fn reuse_cached_compiled_module__wasm_strategy() {
            // Given
            let next_version = Executor::<Storage, DisabledRelayer>::VERSION + 1;
            let storage = storage_with_state_transition(next_version);
            let executor = Executor::wasm(storage, DisabledRelayer, Config::default());
            let block = valid_block(next_version);

            // When
            for _ in 0..1000 {
                let result = executor
                    .execute_without_commit(ExecutionTypes::Validation(block.clone()))
                    .map(|_| ());

                // Then
                assert_eq!(Ok(()), result);
            }
        }
    }
}
