use actix_web::web;
use async_graphql::types::EmptySubscription;
use async_graphql::{Context, Object, Schema};
use async_graphql_actix_web::{Request, Response};
use futures::lock::Mutex;

use std::sync;

use fuel_vm::prelude::*;

pub type TxStorage = sync::Arc<Mutex<MemoryStorage>>;
pub struct MutationRoot;
pub struct QueryRoot;

pub type TXSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

#[Object]
impl QueryRoot {
    async fn version(&self, _ctx: &Context<'_>) -> async_graphql::Result<String> {
        Ok("v1.0".to_owned())
    }
}

#[Object]
impl MutationRoot {
    async fn run(&self, ctx: &Context<'_>, tx: String) -> async_graphql::Result<String> {
        let tx = hex::decode(tx)?;
        let tx = Transaction::from_bytes(tx.as_slice())?;

        let storage = ctx.data_unchecked::<TxStorage>().lock().await;
        let mut vm = Interpreter::with_storage(storage);

        vm.init(tx)?;
        vm.run()?;

        Ok(serde_json::to_string(vm.log())?)
    }
}

pub fn schema() -> TXSchema {
    let storage = TxStorage::default();

    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(storage)
        .finish()
}

pub async fn service(schema: web::Data<TXSchema>, req: Request) -> Response {
    schema.execute(req.into_inner()).await.into()
}
