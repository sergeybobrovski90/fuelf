# Fuel Client

Fuel client implementation.

## Testing

The test suite follows the Rust cargo standards. The GraphQL service will be instantiated by Tower and will emulate a server/client structure.

To run the suite:
`$ cargo test`

## Building

For optimal performance, we recommend using native builds. The generated binary will be optimized for your CPU and may contain specific instructions supported only in your hardware.

To build, run:
`$ RUSTFLAGS="-C target-cpu=native" cargo build --release`

The generated binary will be located in `./target/release/fuel-core`

## Running

The service can listen to an arbitrary socket, as specified in the help command:

```
$ ./target/release/fuel-core --help
fuel-core 0.1.0

USAGE:
    fuel-core [OPTIONS]

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --ip <ip>              [default: 127.0.0.1]
        --port <port>          [default: 4000]
        --db-path <file path>  [default: None]
```

#### Example

```
$ ./target/release/fuel-core --ip 127.0.0.1 --port 4000
Jul 12 23:28:47.238  INFO fuel_core: Binding GraphQL provider to 127.0.0.1:4000
```

#### Log level

The service relies on the environment variable `RUST_LOG`. For more information, check the [env_logger](https://docs.rs/env_logger) crate.

## Docker & Kubernetes
```
# Create Docker Image
ssh-add ~/.ssh/id_ed25519 && DOCKER_BUILDKIT=1 docker build --ssh default -t fuel-core . -f deployment/Dockerfile

ssh-add ~/.ssh/id_rsa && DOCKER_BUILDKIT=1 docker build --ssh default -t fuel-core . -f deployment/Dockerfile

# Delete Docker Image
docker image rm fuel-core

# Create Kubernetes Volume, Deployment & Service
kubectl create -f deployment/fuel-core.yml

# Delete Kubernetes Volume, Deployment & Service
kubectl delete -f deployment/fuel-core.yml
```

## GraphQL service

The client functionality is available through a service endpoint that expect GraphQL queries.

#### Transaction executor

The transaction executor currently performs instant block production. Changes are persisted to RocksDB by default.

* Service endpoint: `/graphql`
* Schema (available after building): `fuel-client/assets/schema.sdl`

The service expects a mutation defined as `submit` that receives a [Transaction](https://github.com/FuelLabs/fuel-tx) in hex encoded binary format, as [specified here](https://github.com/FuelLabs/fuel-specs/blob/master/specs/protocol/tx_format.md).

##### cURL example

This example will execute a script that represents the following sequence of [ASM](https://github.com/FuelLabs/fuel-asm):

```
ADDI(0x10, REG_ZERO, 0xca),
ADDI(0x11, REG_ZERO, 0xba),
LOG(0x10, 0x11, REG_ZERO, REG_ZERO),
RET(REG_ONE),
```

```
$ cargo run --bin fuel-client -- transaction submit \
"{\"Script\":{\"gas_price\":0,\"gas_limit\":1000000,\"maturity\":0,\"script\":[80,64,0,202,80,68,0,186,51,65,16,0,36,4,0,0],\"script_data\":[],\"inputs\":[],\"outputs\":[{\"Coin\":{\"to\":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0], \"amount\": 10, \"color\": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]}}],\"witnesses\":[],\"receipts_root\":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]}}"
```
