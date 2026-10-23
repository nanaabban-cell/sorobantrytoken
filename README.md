# sorobantrytoken

A Soroban NFT contract (ERC-721 / SEP-50 style) in [contracts/nft](contracts/nft).

## Features

- Admin-only `mint(to)` with sequential `u32` token ids
- `transfer`, `transfer_from`, `burn`, `burn_from`
- Per-token `approve` and collection-wide `approve_for_all`, both expiring at a ledger (`0` revokes)
- `name`, `symbol`, `token_uri` (`base_uri` + token id), `balance`, `owner_of`, `total_supply`
- `set_admin`, `set_base_uri`
- Events for mint, transfer, burn, approvals, and admin changes

## Prerequisites

- Rust with the `wasm32v1-none` target: `rustup target add wasm32v1-none`
- Stellar CLI v25.2.0+: `brew install stellar-cli` (or `cargo install --locked stellar-cli`)

## Test

```sh
cargo test
```

## Build

```sh
stellar contract build
# -> target/wasm32v1-none/release/nft.wasm
```

## Deploy to testnet

```sh
stellar keys generate alice --network testnet --fund

stellar contract deploy \
  --wasm target/wasm32v1-none/release/nft.wasm \
  --source alice --network testnet --alias nft \
  -- \
  --admin alice \
  --name "Try Collection" \
  --symbol TRY \
  --base_uri "ipfs://<your-cid>/"
```

## Interact

```sh
stellar contract invoke --id nft --source alice --network testnet -- mint --to alice
stellar contract invoke --id nft --network testnet -- owner_of --token_id 0
stellar contract invoke --id nft --network testnet -- token_uri --token_id 0
```
