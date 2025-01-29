# Saya

Saya is a settlement service for Katana.

## Requirements

- Katana up and running in provable mode (see `katana init`, more to come on that).
- Celestia node up and running that you can send blob to using a celestia token.
- Herodotus Dev account with API key, which can be obtained from https://staging.dashboard.herodotus.dev.
- StarknetOS program compiled from https://github.com/cartridge-gg/snos repository.
- Piltover settlement contract must be deployed on the settlement chain, see [piltover repository](https://github.com/keep-starknet-strange/piltover) or `katana init` can handle it too.
- An account on the settlement chain with funds to verify the proof.

## Environment

For simpler usage, you should export the environment variables required by Saya to run based on the Saya mode / targeted network.

First, check the `.env.example` file and fill in the missing values (some values are pre-filled to settle on Sepolia), copying it to `.env`.

Source the `.env` file:

```bash
export $(grep -v '^#' .env | xargs)
```

You can override any value exported in `.env` by passing the corresponding flag to the `saya` command.

## Persistent mode

```bash
cargo run --bin saya -r -- persistent start
```

## Sovereign mode

In sovereign mode, the genesis block must be provided when chain head has not been persisted yet.

```bash
cargo run --bin saya -r -- sovereign start --genesis.first-block-number <first_block_to_prove>
```
