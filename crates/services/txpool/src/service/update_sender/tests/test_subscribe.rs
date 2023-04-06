use crate::service::update_sender::tests::utils::{
    box_senders,
    MockCreateChannel,
};

use super::{
    utils::senders_strategy_all_ok,
    *,
};

#[derive(Debug, Arbitrary)]
struct Input {
    #[strategy(0..20u8)]
    tx_id: u8,
    #[strategy(senders_strategy_all_ok())]
    senders: HashMap<Bytes32, Vec<Sender<(), MockSendStatus>>>,
}

#[proptest]
fn test_subscriber(input: Input) {
    let Input { tx_id, senders } = input;
    let mut senders = box_senders(senders);
    let len_before = senders.values().map(|v| v.len()).sum::<usize>();
    subscribe::<_, MockCreateChannel>(
        Bytes32::from([tx_id; 32]),
        &mut senders,
        Box::new(()),
    );
    let len_after = senders.values().map(|v| v.len()).sum::<usize>();
    assert_eq!(len_before + 1, len_after);
}
