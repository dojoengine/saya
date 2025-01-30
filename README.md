# Saya

Saya is a settlement service for Katana.

## Requirements

- Katana up and running in provable mode (run `katana init` to generate the chain spec, and then start katana with `katana --chain <CHAIN_ID>` to load the generated parameters).
- Celestia node up and running that you can send blob to using a celestia token.
- Herodotus Dev account with API key, which can be obtained from https://staging.dashboard.herodotus.dev.
- Piltover settlement contract must be deployed on the settlement chain, see [piltover repository](https://github.com/keep-starknet-strange/piltover) or `katana init` can handle it too.
- An account on the settlement chain with funds to verify the proof.

## Cairo programs

Saya currently requires the following Cairo programs to work. Use the scripts in the `scripts` folder to compile them.

```bash
# SNOS
./scripts/generate_snos.sh

# Layout bridge
./scripts/generate_layout_bridge.sh
```

The scripts rely on docker to be installed and available, you may set `SUDO` variable based on your environment:

```bash
SUDO=sudo ./scripts/generate_snos.sh
```

> **_NOTE:_** The `starknet/cairo-lang` docker image is only available for `linux/amd64` architecture, emulation adds a significant overhead to build the `layout_bridge` program, which already requires a large amount of RAM (~32GB).

## Environment

For simpler usage, you should export the environment variables required by Saya to run based on the Saya mode / targeted network.

First, check the `.env.example` file and fill in the missing values (some values are pre-filled to settle on Sepolia), copying it to `.env`.

Source the `.env` file or use:

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
