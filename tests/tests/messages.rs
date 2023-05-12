use fuel_core::{
    chain_config::{
        MessageConfig,
        StateConfig,
    },
    service::{
        Config,
        FuelService,
    },
};
use fuel_core_client::client::{
    types::TransactionStatus,
    FuelClient,
    PageDirection,
    PaginationRequest,
};
use fuel_core_types::{
    fuel_asm::*,
    fuel_crypto::*,
    fuel_merkle,
    fuel_tx::{
        input::message::compute_message_id,
        *,
    },
};
use rstest::rstest;
use std::ops::Deref;

#[cfg(feature = "relayer")]
mod relayer;

#[tokio::test]
async fn messages_returns_messages_for_all_owners() {
    // create some owners
    let owner_a = Address::new([1; 32]);
    let owner_b = Address::new([2; 32]);

    // create some messages for owner A
    let first_msg = MessageConfig {
        recipient: owner_a,
        nonce: 1.into(),
        ..Default::default()
    };
    let second_msg = MessageConfig {
        recipient: owner_a,
        nonce: 2.into(),
        ..Default::default()
    };

    // create a message for owner B
    let third_msg = MessageConfig {
        recipient: owner_b,
        nonce: 3.into(),
        ..Default::default()
    };

    // configure the messages
    let mut config = Config::local_node();
    config.chain_conf.initial_state = Some(StateConfig {
        messages: Some(vec![first_msg, second_msg, third_msg]),
        ..Default::default()
    });

    // setup server & client
    let srv = FuelService::new_node(config).await.unwrap();
    let client = FuelClient::from(srv.bound_address);

    // get the messages
    let request = PaginationRequest {
        cursor: None,
        results: 5,
        direction: PageDirection::Forward,
    };
    let result = client.messages(None, request).await.unwrap();

    // verify that there are 3 messages stored in total
    assert_eq!(result.results.len(), 3);
}

#[tokio::test]
async fn messages_by_owner_returns_messages_for_the_given_owner() {
    // create some owners
    let owner_a = Address::new([1; 32]);
    let owner_b = Address::new([2; 32]);
    let owner_c = Address::new([3; 32]);

    // create some messages for owner A
    let first_msg = MessageConfig {
        recipient: owner_a,
        nonce: 1.into(),
        ..Default::default()
    };
    let second_msg = MessageConfig {
        recipient: owner_a,
        nonce: 2.into(),
        ..Default::default()
    };

    // create a message for owner B
    let third_msg = MessageConfig {
        recipient: owner_b,
        nonce: 3.into(),
        ..Default::default()
    };

    // configure the messages
    let mut config = Config::local_node();
    config.chain_conf.initial_state = Some(StateConfig {
        messages: Some(vec![first_msg, second_msg, third_msg]),
        ..Default::default()
    });

    // setup server & client
    let srv = FuelService::new_node(config).await.unwrap();
    let client = FuelClient::from(srv.bound_address);

    let request = PaginationRequest {
        cursor: None,
        results: 5,
        direction: PageDirection::Forward,
    };

    // get the messages from Owner A
    let result = client
        .messages(Some(&owner_a.to_string()), request.clone())
        .await
        .unwrap();

    // verify that Owner A has 2 messages
    assert_eq!(result.results.len(), 2);

    // verify messages owner matches
    for message in result.results {
        let recipient: Address = message.recipient.into();
        assert_eq!(recipient, owner_a)
    }

    // get the messages from Owner B
    let result = client
        .messages(Some(&owner_b.to_string()), request.clone())
        .await
        .unwrap();

    // verify that Owner B has 1 message
    assert_eq!(result.results.len(), 1);

    let recipient: Address = result.results[0].recipient.into();
    assert_eq!(recipient, owner_b);

    // get the messages from Owner C
    let result = client
        .messages(Some(&owner_c.to_string()), request.clone())
        .await
        .unwrap();

    // verify that Owner C has no messages
    assert_eq!(result.results.len(), 0);
}

#[rstest]
#[tokio::test]
async fn messages_empty_results_for_owner_with_no_messages(
    #[values(PageDirection::Forward)] direction: PageDirection,
    //#[values(PageDirection::Forward, PageDirection::Backward)] direction: PageDirection,
    // reverse iteration with prefix not supported by rocksdb
    #[values(Address::new([16; 32]), Address::new([0; 32]))] owner: Address,
) {
    let srv = FuelService::new_node(Config::local_node()).await.unwrap();
    let client = FuelClient::from(srv.bound_address);

    let request = PaginationRequest {
        cursor: None,
        results: 5,
        direction,
    };

    let result = client
        .messages(Some(&owner.to_string()), request)
        .await
        .unwrap();

    assert_eq!(result.results.len(), 0);
}

#[tokio::test]
async fn can_get_message_proof() {
    for n in [1, 2, 10] {
        let mut config = Config::local_node();
        config.manual_blocks_enabled = true;

        let coin = config
            .chain_conf
            .initial_state
            .as_ref()
            .unwrap()
            .coins
            .as_ref()
            .unwrap()
            .first()
            .unwrap()
            .clone();

        struct MessageArgs {
            recipient_address: [u8; 32],
            message_data: Vec<u8>,
        }

        let args: Vec<_> = (0..n)
            .map(|i| MessageArgs {
                recipient_address: [i + 1; 32],
                message_data: i.to_be_bytes().into(),
            })
            .collect();

        let amount = 10;
        let starting_offset = 32 + 8 + 8;

        let mut contract = vec![
            // Save the ptr to the script data to register 16.
            op::gtf_args(0x10, 0x00, GTFArgs::ScriptData),
            // Offset 16 by the length of bytes for the contract id
            // and two empty params. This will now point to the address
            // of the message recipient.
            op::addi(0x10, 0x10, starting_offset),
        ];
        contract.extend(args.iter().enumerate().flat_map(|(index, arg)| {
            [
                // The length of the message data in memory.
                op::movi(0x11, arg.message_data.len() as u32),
                // The index of the of the output message in the transactions outputs.
                op::movi(0x12, (index + 1) as u32),
                // The amount to send in coins.
                op::movi(0x13, amount),
                // Send the message output.
                op::smo(0x10, 0x11, 0x12, 0x13),
                // Offset to the next recipient address (this recipient address + message data len)
                op::addi(0x10, 0x10, 32 + arg.message_data.len() as u16),
            ]
        }));
        // Return.
        contract.push(op::ret(RegId::ONE));

        // Contract code.
        let bytecode: Witness = contract.into_iter().collect::<Vec<u8>>().into();

        // Setup the contract.
        let salt = Salt::zeroed();
        let contract = Contract::from(bytecode.as_ref());
        let root = contract.root();
        let state_root = Contract::initial_state_root(std::iter::empty());
        let id = contract.id(&salt, &root, &state_root);
        let output = Output::contract_created(id, state_root);

        // Create the contract deploy transaction.
        let contract_deploy = TransactionBuilder::create(bytecode, salt, vec![])
            .add_output(output)
            .finalize_as_transaction();

        let smo_data: Vec<_> = id
            .iter()
            .copied()
            // Empty Param 1
            .chain((0 as Word).to_be_bytes().iter().copied())
            // Empty Param 2
            .chain((0 as Word).to_be_bytes().iter().copied())
            .chain(args.iter().flat_map(|arg| {
                // Recipient address
                arg.recipient_address.into_iter()
                    // The message data
                    .chain(arg.message_data.clone().into_iter())
            })).collect();
        let script_data = AssetId::BASE
            .into_iter()
            .chain(smo_data.into_iter())
            .collect();

        // Call contract script.
        let script = vec![
            // Save the ptr to the script data to register 16.
            // This will be used to read the contract id + two
            // empty params. So 32 + 8 + 8.
            op::gtf_args(0x10, 0x00, GTFArgs::ScriptData),
            // load balance to forward to 0x11
            op::movi(0x11, n as u32 * amount),
            // shift the smo data into 0x10
            op::addi(0x12, 0x10, AssetId::LEN as u16),
            // Call the contract and forward no coins.
            op::call(0x12, 0x11, 0x10, RegId::CGAS),
            // Return.
            op::ret(RegId::ONE),
        ];
        let script: Vec<u8> = script
            .iter()
            .flat_map(|op| u32::from(*op).to_be_bytes())
            .collect();

        let predicate = op::ret(RegId::ONE).to_bytes().to_vec();
        let owner = Input::predicate_owner(&predicate, &ConsensusParameters::DEFAULT);
        let coin_input = Input::coin_predicate(
            Default::default(),
            owner,
            1000,
            coin.asset_id,
            TxPointer::default(),
            Default::default(),
            predicate,
            vec![],
        );

        // Set the contract input because we are calling a contract.
        let inputs = vec![
            Input::contract(
                UtxoId::new(Bytes32::zeroed(), 0),
                Bytes32::zeroed(),
                state_root,
                TxPointer::default(),
                id,
            ),
            coin_input,
        ];

        // The transaction will output a contract output and message output.
        let outputs = vec![Output::Contract {
            input_index: 0,
            balance_root: Bytes32::zeroed(),
            state_root: Bytes32::zeroed(),
        }];

        // Create the contract calling script.
        let script = Transaction::script(
            Default::default(),
            1_000_000,
            Default::default(),
            script,
            script_data,
            inputs,
            outputs,
            vec![],
        );

        let transaction_id = script.id(&ConsensusParameters::DEFAULT);

        // setup server & client
        let srv = FuelService::new_node(config).await.unwrap();
        let client = FuelClient::from(srv.bound_address);

        // Deploy the contract.
        matches!(
            client.submit_and_await_commit(&contract_deploy).await,
            Ok(TransactionStatus::Success { .. })
        );

        // Call the contract.
        matches!(
            client.submit_and_await_commit(&script.into()).await,
            Ok(TransactionStatus::Success { .. })
        );

        // Produce one more block, because we can't create proof for the last block.
        let last_height = client.produce_blocks(1, None).await.unwrap();

        // Get the receipts from the contract call.
        let receipts = client
            .receipts(transaction_id.to_string().as_str())
            .await
            .unwrap()
            .unwrap();

        // Get the message id from the receipts.
        let message_ids: Vec<_> =
            receipts.iter().filter_map(|r| r.message_id()).collect();

        // Check we actually go the correct amount of ids back.
        assert_eq!(message_ids.len(), args.len(), "{receipts:?}");

        for message_id in message_ids.clone() {
            // Request the proof.
            let result = client
                .message_proof(
                    transaction_id.to_string().as_str(),
                    message_id.to_string().as_str(),
                    None,
                    Some(last_height),
                )
                .await
                .unwrap()
                .unwrap();

            // 1. Generate the message id (message fields)
            // Produce message id.
            let generated_message_id = compute_message_id(
                &(result.sender.into()),
                &(result.recipient.into()),
                &(result.nonce.into()),
                result.amount,
                result.data.as_ref(),
            );

            // Check message id is the same as the one passed in.
            assert_eq!(generated_message_id, message_id);

            // 2. Generate the block id. (full header)
            let mut hasher = Hasher::default();
            hasher.input(result.message_block_header.prev_root.as_ref());
            hasher.input(&result.message_block_header.height.to_be_bytes()[..]);
            hasher.input(result.message_block_header.time.0 .0.to_be_bytes());
            hasher.input(result.message_block_header.application_hash.as_ref());
            let message_block_id = hasher.digest();
            assert_eq!(message_block_id, result.message_block_header.id);

            // 3. Verify the message proof. (message receipt root, message id, proof index, proof set, num message receipts in the block)
            let message_proof_index = result.message_proof.proof_index;
            let message_proof_set: Vec<_> = result
                .message_proof
                .proof_set
                .iter()
                .cloned()
                .map(Bytes32::from)
                .collect();
            assert!(verify_merkle(
                result
                    .message_block_header
                    .message_receipt_root
                    .clone()
                    .into(),
                generated_message_id,
                message_proof_index,
                &message_proof_set,
                result.message_block_header.message_receipt_count,
            ));

            // Generate a proof to compare
            let mut tree = fuel_merkle::binary::in_memory::MerkleTree::new();
            for id in &message_ids {
                tree.push(id.as_ref());
            }
            let (expected_root, expected_set) = tree.prove(message_proof_index).unwrap();
            let expected_set: Vec<_> =
                expected_set.into_iter().map(Bytes32::from).collect();

            assert_eq!(message_proof_set, expected_set);

            // Check the root matches the proof and the root on the header.
            assert_eq!(
                <[u8; 32]>::from(Bytes32::from(
                    result.message_block_header.message_receipt_root
                )),
                expected_root
            );

            // 4. Verify the block proof. (prev_root, block id, proof index, proof set, block count)
            let block_proof_index = result.block_proof.proof_index;
            let block_proof_set: Vec<_> = result
                .block_proof
                .proof_set
                .iter()
                .cloned()
                .map(Bytes32::from)
                .collect();
            let blocks_count = result.commit_block_header.height;
            assert!(verify_merkle(
                result.commit_block_header.prev_root.clone().into(),
                message_block_id,
                block_proof_index,
                &block_proof_set,
                blocks_count as u64,
            ));
        }
    }
}

// TODO: Others test:  Data missing etc.
fn verify_merkle<D: AsRef<[u8]>>(
    root: Bytes32,
    data: D,
    index: u64,
    set: &[Bytes32],
    leaf_count: u64,
) -> bool {
    let set: Vec<_> = set.iter().map(|bytes| *bytes.deref()).collect();
    fuel_merkle::binary::verify(root.deref(), data, &set, index, leaf_count)
}
