# Saya

Saya is a settlement service for Katana.

## Binaries

Saya is split into three independent binaries, each in its own Cargo workspace under `bin/`:

| Binary | Workspace | Purpose |
|--------|-----------|---------|
| `persistent` | `bin/persistent/` | Settlement daemon — STARK/Atlantic proof pipeline, Piltover on-chain settlement, and sovereign (Celestia DA) mode |
| `ops` | `bin/ops/` | Ops utilities — Piltover contract deployment and management, Celestia helpers |
| `persistent-tee` | `bin/persistent-tee/` | TEE settlement daemon — AMD SEV-SNP attestation → SP1 proof → Piltover on-chain settlement |

`saya/core` is a shared infrastructure library (no STARK or TEE-specific deps) used by all binaries.

---

## Katana provable mode

Katana must be running in **provable mode** to be proven by Saya.
All commands described below are available starting from **Dojo `1.3.0`**.

### Important limitation

Provable Katana currently supports only **Starknet v14.0.1**.
Because of this, Katana's built-in logic for deploying the core contract **cannot be used**.
Instead, deployment is handled by the **`ops` binary**.

### Correct flow

1. Use the `ops` binary to:
   - declare (or use the predeclared) core contract,
   - deploy the contract,
   - set the program info and fact registry.

2. From this process, obtain:
   - the **core contract address**,
   - the **block number** where it was deployed.

3. Use these values when running `katana init`.

---

## Ops binary (`bin/ops`)

### Required environment variables

```bash
export SETTLEMENT_ACCOUNT_PRIVATE_KEY=<PRIVATE_KEY_IN_HEX>
export SETTLEMENT_ACCOUNT_ADDRESS=<ACCOUNT_ADDRESS_IN_HEX>
export SETTLEMENT_CHAIN_ID=<sepolia|mainnet|CUSTOM_CHAIN_ID>
```

`SETTLEMENT_RPC_URL` is optional — if `SETTLEMENT_CHAIN_ID` is `sepolia` or `mainnet` the default public RPC is used automatically.

### Declare the core contract

```bash
cargo run --manifest-path bin/ops/Cargo.toml -- core-contract declare
```

Or with a custom contract path:

```bash
cargo run --manifest-path bin/ops/Cargo.toml -- core-contract declare --core-contract-path <PATH>
```

Expected output for the unmodified core contract:

```
[INFO  saya::core_contract::utils] Core contract already declared on-chain.
[INFO  saya::core_contract::cli] Core contract class hash: 0x5aed647bf20ab45d4ca041823019ab1f98425eba797ce6b998af94237677f5
```

### Deploy the core contract

```bash
cargo run --manifest-path bin/ops/Cargo.toml -- core-contract deploy --salt 0x5
```

The output contains two important values — **block number** and **contract address** — save them for future use:

```
[INFO  saya::core_contract::utils] Core contract deployed.
[INFO  saya::core_contract::utils]  Tx hash   : 0x5bfedaba61dcebb3ab0a8f5856eace3a6bec17f007654142f5004b0ef4f39bf
[INFO  saya::core_contract::utils]  Deployed on block  : 6180778
[INFO  saya::core_contract::cli] Core contract address: 0x9da87cf1e8ceccb46e7d044541b51bc7f369c262f332e49152e74b30659b53
```

Options:

```
--class-hash <CLASS_HASH>  [env: CLASS_HASH=]  [default: latest Piltover hash]
--salt <SALT>              [env: SALT=]
```

### Set program info and fact registry

```bash
cargo run --manifest-path bin/ops/Cargo.toml -- core-contract setup-program \
  --core-contract-address 0x9da87cf1e8ceccb46e7d044541b51bc7f369c262f332e49152e74b30659b53 \
  --chain-id example-chain
```

Options:

```
--fact-registry-address <FACT_REGISTRY_ADDRESS>  [env: FACT_REGISTRY_ADDRESS=]
--core-contract-address <CORE_CONTRACT_ADDRESS>  [env: CORE_CONTRACT_ADDRESS=]
--fee-token-address <FEE_TOKEN_ADDRESS>          [env: FEE_TOKEN_ADDRESS=]
--chain-id <CHAIN_ID>                            [env: CHAIN_ID=]
```

### Deploy a mock fact registry (for testing)

Deploys a fact registry that confirms any submitted fact — useful for local testing without real proofs:

```bash
cargo run --manifest-path bin/ops/Cargo.toml -- core-contract declare-and-deploy-fact-registry-mock --salt 0x1
```

Then pass the deployed address to `setup-program --fact-registry-address`.

---

## Initialize Katana

`katana init` generates a configuration file and genesis block:

```bash
katana init \
  --settlement-chain sepolia \
  --id example-chain \
  --settlement-contract 0x9da87cf1e8ceccb46e7d044541b51bc7f369c262f332e49152e74b30659b53 \
  --settlement-contract-deployed-block 6180778
```

Use `katana config` to list local configurations.

Start Katana with:

```bash
katana --chain <CHAIN_ID> --block-time 30000
```

> **Note:** When running Katana in provable mode it is recommended to set a block time (in milliseconds). Katana starts the block timer on the first transaction in a block and never produces empty blocks.

> **Note:** Use `--output-path` with `katana init` to write config to a custom directory, then start with `--chain /path`.

---

## Requirements

- Katana running in provable mode.

### Persistent mode

- Piltover settlement contract deployed on the settlement chain (via `ops core-contract` above, or `katana init`).
- An account on the settlement chain with funds to submit settlement transactions.
- Herodotus Atlantic API key (from <https://herodotus.cloud>) unless using `--mock-layout-bridge-program-hash`.

### Persistent-TEE mode

- Katana TEE node (via `katana-tee`) running and reachable.
- TEE registry contract deployed on the prover network.
- An account on the prover network for submitting proofs.
- An account on the settlement chain with funds to submit settlement transactions.

### Sovereign mode

- Celestia light node running with a funded account for blob submission. A helper script is available at `scripts/celestia.sh`.

---

## Cairo programs

Persistent mode requires a compiled `layout_bridge` Cairo program:

```bash
./scripts/generate_layout_bridge.sh
```

The script requires Docker. Set `SUDO` if needed:

```bash
SUDO=sudo ./scripts/generate_layout_bridge.sh
```

> **Note:** The `starknet/cairo-lang` Docker image is `linux/amd64` only — emulation requires ~32 GB RAM.

> **Note:** Pre-built programs are available in the Saya Docker image at `/programs` and on the [Saya release page](https://github.com/dojoengine/saya/releases).

---

## Persistent mode

### Running

```bash
cargo run --manifest-path bin/persistent/Cargo.toml -r -- start \
  --rollup-rpc http://localhost:5050 \
  --settlement-rpc https://starknet-sepolia.public.blastapi.io \
  --settlement-piltover-address <PILTOVER_ADDRESS> \
  --settlement-account-address <ACCOUNT_ADDRESS> \
  --settlement-account-private-key <PRIVATE_KEY> \
  --layout-bridge-program programs/layout_bridge.json \
  --atlantic-key <ATLANTIC_KEY> \
  --settlement-integrity-address <INTEGRITY_ADDRESS>
```

Key options:

```
--rollup-rpc <URL>                           Katana L3 JSON-RPC endpoint
--settlement-rpc <URL>                       Settlement chain JSON-RPC endpoint
--settlement-piltover-address <FELT>         Piltover contract address
--settlement-account-address <FELT>          Submitter account address
--settlement-account-private-key <FELT>      Submitter account private key
--layout-bridge-program <PATH>               Path to compiled layout_bridge program
--atlantic-key <KEY>                         Atlantic (Herodotus) API key
--settlement-integrity-address <FELT>        On-chain integrity/fact registry address
--blocks-processed-in-parallel <N>           Parallel block pipeline depth (default: 60)
--db-dir <PATH>                              SQLite database directory
```

### Mocking proofs (for testing)

Replace the layout bridge proof with a mock (skips on-chain fact registration) by first switching Piltover to a mock fact registry:

```bash
cargo run --manifest-path bin/ops/Cargo.toml -- core-contract setup-program \
  --core-contract-address <PILTOVER_ADDRESS> \
  --chain-id example-chain \
  --fact-registry-address <MOCK_FACT_REGISTRY_ADDRESS>
```

Then start with:

```bash
cargo run --manifest-path bin/persistent/Cargo.toml -r -- start \
  ... \
  --mock-layout-bridge-program-hash 0x43c5c4cc37c4614d2cf3a833379052c3a38cd18d688b617e2c720e8f941cb8
```

To also mock the SNOS proof (derives it from the PIE):

```bash
cargo run --manifest-path bin/persistent/Cargo.toml -r -- start \
  ... \
  --mock-layout-bridge-program-hash 0x43c5c4cc37c4614d2cf3a833379052c3a38cd18d688b617e2c720e8f941cb8 \
  --mock-snos-from-pie
```

---

## Sovereign mode

```bash
cargo run --manifest-path bin/persistent/Cargo.toml -r -- sovereign start \
  --rollup-rpc http://localhost:5050 \
  --celestia-rpc http://localhost:26658 \
  --celestia-token <TOKEN>
```

---

## Persistent-TEE mode

The TEE pipeline ingests blocks from Katana, fetches an AMD SEV-SNP attestation quote, generates an SP1 Groth16 proof via the TEE registry, and settles on Piltover.

### Running

```bash
cargo run --manifest-path bin/persistent-tee/Cargo.toml -r -- tee start \
  --rollup-rpc http://localhost:5050 \
  --settlement-rpc https://starknet-sepolia.public.blastapi.io \
  --settlement-piltover-address <PILTOVER_ADDRESS> \
  --settlement-account-address <ACCOUNT_ADDRESS> \
  --settlement-account-private-key <PRIVATE_KEY> \
  --tee-registry-address <TEE_REGISTRY_ADDRESS> \
  --prover-private-key <PROVER_PRIVATE_KEY>
```

Key options:

```
--rollup-rpc <URL>                       Katana TEE node JSON-RPC endpoint
--settlement-rpc <URL>                   Settlement chain JSON-RPC endpoint
--settlement-piltover-address <FELT>     Piltover contract address
--settlement-account-address <FELT>      Submitter account address
--settlement-account-private-key <FELT>  Submitter account private key
--tee-registry-address <FELT>            TEE registry contract on the prover network
--prover-private-key <STRING>            Prover network account private key
--batch-size <N>                         Blocks per attestation batch (default: 10)
--idle-timeout-secs <N>                  Flush partial batch after N idle seconds (default: 120)
--attestor-poll-interval-ms <N>          Attestor poll interval in ms (default: 1000)
--db-dir <PATH>                          SQLite database directory
```

---

## Testing

```bash
# Unit + integration tests
cargo test --workspace --all-features

# E2E (requires Docker)
docker compose -f compose.e2e.yml up
```
