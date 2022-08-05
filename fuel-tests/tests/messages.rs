use fuel_core::{config::Config, database::Database, service::FuelService};
use fuel_core_interfaces::common::fuel_storage::Storage;
use fuel_core_interfaces::model::DaMessage;
use fuel_crypto::fuel_types::{Address, MessageId};
use fuel_gql_client::client::FuelClient;

#[tokio::test]
async fn messages() {
    // setup server & client
    let mut db = Database::default();
    let srv = FuelService::from_database(db.clone(), Config::local_node())
        .await
        .unwrap();
    let _client = FuelClient::from(srv.bound_address);

    // create some owners
    let owner_a = Address::new([1; 32]);
    let owner_b = Address::new([2; 32]);

    // create some messages for owner A
    let mut first_msg = DaMessage::default();
    first_msg.owner = owner_a;
    let mut second_msg = DaMessage::default();
    second_msg.owner = owner_a;

    // create a message for owner B
    let mut third_msg = DaMessage::default();
    third_msg.owner = owner_b;

    // store the messaages
    let first_id = MessageId::new([1; 32]);
    let _ = Storage::<MessageId, DaMessage>::insert(&mut db, &first_id, &first_msg).unwrap();

    let second_id = MessageId::new([2; 32]);
    let _ = Storage::<MessageId, DaMessage>::insert(&mut db, &second_id, &second_msg).unwrap();

    let third_id = MessageId::new([3; 32]);
    let _ = Storage::<MessageId, DaMessage>::insert(&mut db, &third_id, &third_msg).unwrap();

    // get the messages

    // get the messages by owner
}
