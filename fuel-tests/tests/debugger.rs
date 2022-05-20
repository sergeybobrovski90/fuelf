use fuel_core::service::{Config, FuelService};
use fuel_gql_client::client::FuelClient;
use fuel_vm::prelude::*;

#[tokio::test]
async fn debugger() {
    let srv = FuelService::new_node(Config::local_node()).await.unwrap();
    let client = FuelClient::from(srv.bound_address);

    let session = client.start_session().await.unwrap();
    let session_id = session.as_str();

    let register = client.register(session_id, 0x10).await.unwrap();
    assert_eq!(0x00, register);

    client
        .set_breakpoint(session_id, Breakpoint::script(0))
        .await
        .unwrap();

    let tx: Transaction =
        serde_json::from_str(include_str!("example_tx.json")).expect("Invalid transaction JSON");
    let status = client.start_tx(session_id, &tx).await.unwrap();
    assert!(status.breakpoint.is_some());

    client.set_single_stepping(session_id, true).await.unwrap();

    let status = client.continue_tx(session_id).await.unwrap();
    assert!(status.breakpoint.is_some());

    client.set_single_stepping(session_id, false).await.unwrap();

    let status = client.continue_tx(session_id).await.unwrap();
    assert!(status.breakpoint.is_none());

    let result = client.end_session(session_id).await.unwrap();
    assert!(result);
}
