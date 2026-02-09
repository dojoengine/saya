# Saya

Saya is a settlement service for Katana.

## Katana provable mode

Katana must be running in **provable mode** to be proven by Saya.
All commands described below are available starting from **Dojo `1.3.0`**.

### Important limitation

Provable Katana currently supports only **Starknet v14.0.1**.
Because of this, Katana’s built-in logic for deploying the core contract **cannot be used**.
Instead, deployment is handled by **Saya**.

### Correct flow

1. Use Saya to:

   * declare (or use the predeclared) core contract,
   * deploy the contract,
   * set the program info and fact registry.

2. From this process, obtain:

   * the **core contract address**,
   * the **block number** where it was deployed.

3. Use these values when running `katana init`.

---

## Core contract (Piltover)

The `core-contract` subcommand manages the Piltover core contract:

* declaring the class,
* deploying the contract,
* setting program info and fact registry.

### Required environment variables

```bash
export SETTLEMENT_ACCOUNT_PRIVATE_KEY=<PRIVATE_KEY_IN_HEX>
export SETTLEMENT_ACCOUNT_ADDRESS=<ACCOUNT_ADDRESS_IN_HEX>
export SETTLEMENT_CHAIN_ID=<STRING_CHAIN_ID>
```

> This step is optional if you rely on the default class hash used by the `deploy` command (latest compatible Piltover).

---

## Declare the core contract

The only argument is the path to the core contract definition.
By default, it points to the contract in the repository:

```
--core-contract-path <CORE_CONTRACT_PATH>
    [env: CORE_CONTRACT_PATH=]
    [default: contracts/core_contract.json]
```

Command:

```bash
cargo run core-contract declare
```

or:

```bash
cargo run core-contract declare --core-contract-path <PATH>
```

Expected output for the unmodified core contract:

```
[INFO  saya::core_contract::utils] Core contract already declared on-chain.
[INFO  saya::core_contract::cli] Core contract class hash: 0x5aed647bf20ab45d4ca041823019ab1f98425eba797ce6b998af94237677f5
```

---

## Deploy the core contract

The deploy command accepts the following options:

```
--class-hash <CLASS_HASH>  [env: CLASS_HASH=]
                           [default: latest Piltover hash]
--salt <SALT>             [env: SALT=]
```

By default, the class hash is set to the latest compatible Piltover core contract.

Example:

```bash
cargo run core-contract deploy --salt 0x5
```

The output contains two important values:

* **block number**,
* **contract address**.

So save them for future use.

Example output:

```
[INFO  saya::core_contract::utils] Core contract deployed.
[INFO  saya::core_contract::utils]  Tx hash   : 0x5bfedaba61dcebb3ab0a8f5856eace3a6bec17f007654142f5004b0ef4f39bf
[INFO  saya::core_contract::utils]  Deployed on block  : 6180778
[INFO  saya::core_contract::cli] Core contract address: 0x9da87cf1e8ceccb46e7d044541b51bc7f369c262f332e49152e74b30659b53
```

---

## Set program info and fact registry

The last step is to set the program info and fact registry (defaults to the Atlantic fact registry).

Options:

```
--fact-registry-address <FACT_REGISTRY_ADDRESS>
    [env: FACT_REGISTRY_ADDRESS=]

--core-contract-address <CORE_CONTRACT_ADDRESS>
    [env: CORE_CONTRACT_ADDRESS=]

--fee-token-address <FEE_TOKEN_ADDRESS>
    [env: FEE_TOKEN_ADDRESS=]
    [default: 0x2e7442625bab778683501c0eadbc1ea17b3535da040a12ac7d281066e915eea]

--chain-id <CHAIN_ID>
    [env: CHAIN_ID=]
```

Example:

```bash
cargo run core-contract setup-program \
  --core-contract-address 0x9da87cf1e8ceccb46e7d044541b51bc7f369c262f332e49152e74b30659b53 \
  --chain-id example-chain
```

Example output:

```
[INFO  saya::core_contract::cli] Starknet OS config hash: 0x1676f3cc88a3ac2bf40e1a6780b73c46ccd5769e0e141ffc5491981a131e5d5
[INFO  saya::core_contract::cli] Set program info transaction submitted: Hash(0x4a7dbe9d4c8613518acde5d66b69f7daa29efe756c20182b01b83856aed0cda)
[INFO  saya::core_contract::cli] Fact registry set transaction submitted: Hash(0x27c24c82c0f1b7ac71cba02d4cbf1aa0136094d2bd9f9396b1f3f78afecc167)
```

---

## Initialize Katana

`katana init` generates:

* a configuration file,
* a genesis block.

Example:

```bash
katana init \
  --settlement-chain sepolia \
  --id example-chain \
  --settlement-contract 0x9da87cf1e8ceccb46e7d044541b51bc7f369c262f332e49152e74b30659b53 \
  --settlement-contract-deployed-block 6180778 
  
```

Use `katana config` to list all the local configuration and `katana config <CHAIN_ID>` to display the configuration and the file path if you want to inspect it.

1. Start Katana with `katana --chain <CHAIN_ID>` to load the generated parameters at start.

> **_NOTE:_** You can define an `--output-path` when working with `katana init` to output the configuration files in the given directory. You will then want to start katana with the `--chain /path` instead of `--chain <CHAIN_ID>`.

1. Block time: when running Katana in provable mode, the block time is important, since each block will be proven by Saya, and eventually settled or posted to a data availability layer (which in both cases is incurring an additional cost).

   It is then recommended to run Katana with a block time. It is important to note that Katana is starting the block time for the very first transaction received for the block, and will never produce empty blocks.

   ```bash
   # Example for Katana with a block time of 30 seconds.
   katana --chain <CHAIN_ID> --block-time 30000
   ```

## Requirements

* Katana up and running in provable mode.
* Herodotus Dev account with API key, which can be obtained from <https://herodotus.cloud>.

### Sovereign mode

* Celestia node up and running that you can send blob to using a celestia token (only for sovereign mode at the moment). A script is available in `scripts/celestia.sh` to help with the setup.
* An account to send the blobs (usually configured with the light node you are running).

### Persistent mode

* Piltover settlement contract must be deployed on the settlement chain, see [piltover repository](https://github.com/keep-starknet-strange/piltover) or `katana init` can handle it too.
* An account on the settlement chain with funds to verify the proof.

## Cairo programs

Saya currently requires the following Cairo program to work. Use the script in the `scripts` folder to compile it.

```bash
./scripts/generate_layout_bridge.sh
```

The scripts rely on docker to be installed and available, you may set `SUDO` variable based on your environment:

```bash
SUDO=sudo ./scripts/generate_layout_bridge.sh
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
cargo run core-contract --settlement-chain-id sepolia setup-program   --core-contract-address <PILTOVER_ADDRESS>   --chain-id example-chain  --fact-registry-address 0x01eda48cc753670a9a00313afd08bac6e1606943d554ea4a6040cd2953d67867```

Then you can run Saya with:

```

saya persistent start \
    --mock-layout-bridge-program-hash 0x43c5c4cc37c4614d2cf3a833379052c3a38cd18d688b617e2c720e8f941cb8

```

By doing so, Saya will mock the layout bridge proof and call the `update_state` function of the settlement contract.

In order to also mock the SNOS proof, you can use the following command:

```

saya persistent start \
    --mock-layout-bridge-program-hash 0x43c5c4cc37c4614d2cf3a833379052c3a38cd18d688b617e2c720e8f941cb8 \
    --mock-snos-from-pie

```

This will generates the SNOS's PIE, and mock the proof from it.
