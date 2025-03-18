# Saya

Saya is a settlement service for Katana.

## Katana provable mode

Katana must be running in provable mode to be proven by Saya.
All the following Katana commands are available from Dojo `1.3.0` and above.

1. Use `katana init` to setup the chain spec, you can use the options or the prompt (default) to setup the chain spec.

```
katana init
```

2. `katana init` generates a directory with configuration file and genesis block. Use `katana config` to list all the local configuration and `katana config <CHAIN_ID>` to display the configuration and the file path if you want to inspect it.

3. Start Katana with `katana --chain <CHAIN_ID>` to load the generated parameters at start.

> **_NOTE:_** You can define an `--output-path` when working with `katana init` to output the configuration files in the given directory. You will then want to start katana with the `--chain /path` instead of `--chain <CHAIN_ID>`.

> **_NOTE:_** If piltover settlement contract is already deployed, you can skip the automatic deployment by using the `--settlement-contract` and providing the contract address.

4. Block time: when running Katana in provable mode, the block time is important, since each block will be proven by Saya, and eventually settled or posted to a data availability layer (which in both cases is incurring an additional cost).

   It is then recommended to run Katana with a block time. It is important to note that Katana is starting the block time for the very first transaction received for the block, and will never produce empty blocks.

   ```bash
   # Example for Katana with a block time of 30 seconds.
   katana --chain <CHAIN_ID> --block-time 30000
   ```

5. Block step limitation: Due to an issue in the CairoVM not yet merged in Katana, to ensure that the block is provable by Saya, the maximum cairo steps in a block must be at most `16_000_000`. If this limit is reached, Katana will mined the block (regardless of the block time).

   Once this limitation will be removed, the max cairo steps will be `40_000_000` (already enforced by Katana internally).

   ```bash
   katana --chain <CHAIN_ID> --block-time 30000 --sequencing.block-max-cairo-steps 16000000
   ```

## Requirements

- Katana up and running in provable mode.
- Herodotus Dev account with API key, which can be obtained from https://herodotus.cloud.

### Sovereign mode

- Celestia node up and running that you can send blob to using a celestia token (only for sovereign mode at the moment). A script is available in `scripts/celestia.sh` to help with the setup.
- An account to send the blobs (usually configured with the light node you are running).

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

> **_NOTE:_** If you don't have a machine that can compile the programs, you can find them in the Saya docker image mounted in the `/programs` directory, or they can be downloaded from the [Saya release page](https://github.com/dojoengine/saya/releases).

## Environment

For simpler usage, you should export the environment variables required by Saya to run based on the Saya mode / targeted network.

First, check the `.env.persistent.example` or `.env.sovereign.example` file and fill in the missing values (some values are pre-filled to settle on Sepolia), copying it to `.env.persistent` or `.env.sovereign`.

Source the `.env.persistent` or `.env.sovereign` file or use:

```bash
# Persistent
export $(grep -v '^#' .env.persistent | xargs)

# Sovereign
export $(grep -v '^#' .env.sovereign | xargs)
```

You can override any value exported in `.env.persistent` or `.env.sovereign` by passing the corresponding flag to the `saya` command.

Those files are into the `.gitignore` file, so they are not checked into the repository.

## Persistent mode

```bash
cargo run --bin saya -r -- persistent start
```

## Sovereign mode

```bash
cargo run --bin saya -r -- sovereign start
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

In order to also mock the SNOS proof, you can use the following command:

```
saya persistent start \
    --mock-layout-bridge-program-hash 0x193641eb151b0f41674641089952e60bc3aded26e3cf42793655c562b8c3aa0 \
    --mock-snos-from-pie
```

This will generates the SNOS's PIE, and mock the proof from it.
