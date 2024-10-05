use crate::client::schema::{
    schema,
    BlobId,
    HexString,
};

#[derive(cynic::QueryVariables, Debug)]
pub struct BlobByIdArgs {
    pub id: BlobId,
}

#[derive(cynic::QueryFragment, Clone, Debug)]
#[cynic(
    schema_path = "./assets/schema.sdl",
    graphql_type = "Query",
    variables = "BlobByIdArgs"
)]
pub struct BlobByIdQuery {
    #[arguments(id: $id)]
    pub blob: Option<Blob>,
}

#[derive(cynic::QueryFragment, Clone, Debug)]
#[cynic(schema_path = "./assets/schema.sdl")]
pub struct Blob {
    pub id: BlobId,
    pub bytecode: HexString,
}

#[derive(cynic::QueryFragment, Clone, Debug)]
#[cynic(schema_path = "./assets/schema.sdl", graphql_type = "Blob")]
pub struct BlobIdFragment {
    pub id: BlobId,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct BlobExistsArgs {
    pub id: BlobId,
}

#[derive(cynic::QueryFragment, Clone, Debug)]
#[cynic(
    schema_path = "./assets/schema.sdl",
    graphql_type = "Query",
    variables = "BlobExistsArgs"
)]
pub struct BlobExistsQuery {
    #[arguments(id: $id)]
    pub blob: Option<BlobIdFragment>,
}
