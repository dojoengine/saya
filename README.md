# Saya

Saya is a settlement service for Katana.

## Katana provable mode

Katana must be running in provable mode to be proven by Saya.
All the following Katana commands are available from Dojo `1.2.0` and above.

1. Use `katana init` to setup the chain spec, you can use the prompt or the following arguments:

```
 katana init \
    --id <CHAIN_ID> \
    --settlement-chain Sepolia \
    --settlement-account-address <SEPOLIA_ACCOUNT> \
    --settlement-account-private-key <PRIVATE_KEY>
```

This will create a chain spec in the [Katana's configuration directory](https://github.com/dojoengine/dojo/blob/5e1f3b93e769d135b7a01d3c7e648cc9e0f7e7fa/crates/katana/chain-spec/src/rollup/file.rs#L272) and deploy the settlement contract.

2. `katana init` generates a directory with configuration file and genesis block. Use `katana init --show-config <CHAIN_ID>` to display the configuration file path if you want to inspect it.

3. Start Katana with `katana --chain <CHAIN_ID>` to load the generated parameters at start.

> **_NOTE:_** You can define an `--output-path` when working with `katana init` to output the configuration files in the given directory. You will then want to start katana with the `--chain /path` instead of `--chain <CHAIN_ID>`.

> **_NOTE:_** If piltover settlement contract is already deployed, you can skip the automatic deployment by using the `--settlement-contract` and providing the contract address.

## Requirements

- Katana up and running in provable mode.
- Herodotus Dev account with API key, which can be obtained from https://staging.dashboard.herodotus.dev.

### Sovereign mode

- Celestia node up and running that you can send blob to using a celestia token (only for sovereign mode at the moment).

### Persistent mode

- Piltover settlement contract must be deployed on the settlement chain, see [piltover repository](https://github.com/keep-starknet-strange/piltover) or `katana init` can handle it too.
- An account on the settlement chain with funds to verify the proof.

## Cairo programs

Saya currently requires the following Cairo programs to work. Use the scripts in the `scripts` folder to compile them.

```bash
./scripts/generate_snos.sh

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

## Testing

Since persistent mode requires two proofs (SNOS and Layout bridge), you can opt to mock the layout bridge proof by providing the `--mock-layout-bridge-program-hash` argument for testing purposes.

Before running Saya, you must first change the fact registry address for the piltover settlement contract to use a mock one.

> **_NOTE:_** `0x01eda48cc753670a9a00313afd08bac6e1606943d554ea4a6040cd2953d67867` is a deployed mock fact registry address on Sepolia that returns the expected fact confirmation for any fact.

```
starkli invoke <PILTOVER_ADDRESS> set_facts_registry 0x01eda48cc753670a9a00313afd08bac6e1606943d554ea4a6040cd2953d67867
```

Then you can run Saya with:

```
saya persistent start \
    --mock-layout-bridge-program-hash 0x193641eb151b0f41674641089952e60bc3aded26e3cf42793655c562b8c3aa0
```

By doing so, Saya will mock the layout bridge proof and call the `update_state` function of the settlement contract.
