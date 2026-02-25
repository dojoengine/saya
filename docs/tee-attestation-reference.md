# TEE Attestation Reference

> Derived from analysis of `cartridge-gg/sharding-operator` (`/home/mateusz/dev/sharding-cart`).
> Saya's use case is simpler: **no storage slots, no sharding proxy** — only block attestation → proof → settlement.

---

## 1. High-Level Pipeline (sharding-operator)

```
Katana TEE Instance
        │
        ▼  KatanaTeeClient::fetch_attestation() + fetch_quote_bytes()
   TeeAttestation { quote_bytes[1184], state_root, block_hash, block_number }
        │
        ▼  StorageProofFetcher::fetch_and_verify()         ← NOT NEEDED for saya
   StorageProofData + Verified<StorageValues>              ← NOT NEEDED for saya
        │
        ▼  verify_attestation_consistency()
   Cross-check: block_hash == proof.block_hash
                state_root == Poseidon("STARKNET_STATE_V0", contracts_root, classes_root)
        │
        ▼  TeeAttestation::generate_proof_with_storage()
   OnchainProof (SP1 Groth16)
        │
        ▼  proof_to_calldata() → Vec<Felt>
        │
        ▼  build_settlement_calls()
   Call 1: KatanaTee.verify_and_update_state(proof, state_root, block_hash, block_number)
   Call 2: Sharding.update_contract_state_tee(...)        ← NOT NEEDED for saya
        │
        ▼  account.execute_v3(calls)
   Transaction hash
```

**For saya (no storage slots):** only Call 1 matters. The SP1 circuit still needs
a `StorageProofParams`, but keys/values/proof_nodes can all be empty.

---

## 2. Katana TEE RPC Endpoints

### 2.1 Fetch Attestation

No standard JSON-RPC method name exposed — calls are made via `KatanaTeeClient`
from the `katana_tee_client` crate (see §5).

**What it returns (`TeeQuoteResponse`):**
```
quote_bytes:   Vec<u8>  — 1184 bytes for AMD SEV-SNP
state_root:    Felt
block_hash:    Felt
block_number:  u64
```

**Saya stub:** `TeeAttestor::fetch_attestation()` in
`saya/core/src/tee/attestor.rs` currently returns an empty placeholder.
Real impl should call `KatanaRpcClient::fetch_attestation()`.

### 2.2 `starknet_getStorageProof` (NOT NEEDED for saya)

Custom Katana extension. Returns Merkle-Patricia tree proofs for specified
storage slots. Only required when you need to prove storage values on-chain.

---

## 3. TEE Attestation Data Structure

```rust
// sharding-cart: src/tee/attestation.rs:54-75
struct TeeAttestation {
    quote_bytes:  Vec<u8>,  // AMD SEV-SNP quote, exactly 1184 bytes
    state_root:   Felt,     // Starknet global state root (contracts+classes Poseidon hash)
    block_hash:   Felt,     // Katana block hash
    block_number: u64,
}
```

The `quote_bytes` are an AMD SEV-SNP attestation report. The report contains a
custom user-data field where Katana embeds the block hash so the TEE quote
cryptographically binds the hardware execution to the specific Starknet block.

---

## 4. SP1 Proof Generation

### 4.1 Where it happens

```
src/tee/attestation.rs:139-260  generate_proof_with_storage()
```

### 4.2 Steps inside `generate_proof_with_storage()`

1. Parse `AttestationReportBytes` → `AttestationReport`
2. Detect processor model (`Milan / Genoa / Bergamo / Siena`)
3. Fetch KDS cert chain via `KDS::new().fetch_report_cert_chain()`
4. Parse cert chain with `CertChain::parse_rev()`
5. Get current timestamp
6. Create `StarknetRegistryClient` for provider URL
7. Fetch `trusted_prefix_len` from AMD registry contract (on-chain)
8. Validate cert chain time (unless `skip_time_validity_check`)
9. Build `SP1ProverConfig` (mode: network / mock / local)
10. Spawn `AmdSevSnpProver` in blocking task
11. Call `prepare_verifier_input_with_storage()` — builds ZK circuit input
12. Generate Groth16 proof via `prover.verifier.gen_proof()`
13. Convert to `OnchainProof` via `create_onchain_proof()`

Timeout: **600 seconds** enforced via tokio.

### 4.3 SP1ProofInput (circuit input)

```rust
// src/protocol/pipeline.rs:93-99
struct Sp1ProofInput {
    attestation_quote: Vec<u8>,          // 1184 bytes
    block_state:       BlockState,        // { block_number, block_hash, state_root }
    storage_proof:     StorageProofData,  // empty for saya
    verified_values:   Verified<StorageValues>,  // empty for saya
    event_proof:       EventInclusionProof,       // placeholder, not yet implemented
}
```

### 4.4 StorageProofParams (empty for saya)

```rust
// amd_tee_registry_client::StorageProofParams
StorageProofParams {
    global_state_root:       B256,
    contracts_tree_root:     B256,
    classes_tree_root:       B256,
    contract_storage_root:   B256,
    contract_class_hash:     B256,
    contract_leaf_nonce:     u64,
    keys:                    Vec<Bytes>,  // empty for saya
    values:                  Vec<Bytes>,  // empty for saya
    storage_proof_nodes:     Vec<Bytes>,  // empty for saya
    contracts_proof_nodes:   Vec<Bytes>,  // can be empty for saya
    nonce:                   u64,         // replay-protection nonce from registry
}
```

For saya the `keys`, `values`, `storage_proof_nodes` fields are all empty.
The `global_state_root`, `contracts_tree_root`, `classes_tree_root` still need
to be populated from the block state so the SP1 circuit can verify the TEE quote
binds to the correct block.

---

## 5. External Crates (all from `feltroidprime/katana-tee`)

All from `ssh://git@github.com/feltroidprime/katana-tee.git`, branch `feature/sharding`.

| Crate | Purpose |
|-------|---------|
| `katana_tee_client` | `KatanaRpcClient`, `TeeQuoteResponse`, `ProverConfig`, `OnchainProof` |
| `amd-sev-snp-attestation-prover` | `AmdSevSnpProver`, `SP1ProverConfig`, `KDS` — generates the Groth16 proof |
| `amd-sev-snp-attestation-verifier` | `AttestationReport`, `ProcessorType` — parses raw quote bytes |
| `amd_tee_registry_client` | `StarknetRegistryClient`, `StorageProofParams` — fetches trusted_prefix_len |
| `x509-verifier-rust-crypto` | `CertChain` — parses and validates KDS certificate chain |

Other TEE-specific crates:

| Crate | Source | Purpose |
|-------|--------|---------|
| `bonsai-trie` | github.com/dojoengine/bonsai-trie rev 351d5be | Merkle proof verification (NOT NEEDED for saya) |
| `alloy-primitives` | 1.3.1 | `B256` type for storage proof params |
| `parity-scale-codec` | 3 | SCALE encoding for Merkle nodes (NOT NEEDED for saya) |

All `alloy` crates must be patched to `v1.0.41` for katana-tee compatibility:
```toml
[patch.crates-io]
alloy = { git = "https://github.com/alloy-rs/alloy", tag = "v1.0.41" }
# ... ~15 alloy sub-crates
```

---

## 6. On-Chain Settlement Call (what saya needs)

```rust
// selector: "verify_and_update_state"
// calldata layout (sharding-operator src/shard/calls.rs:111-134):
[
    proof_felts.len(),   // u32 as Felt
    proof_felts...,      // Groth16 proof as Felt array
    state_root,          // Felt
    block_hash,          // Felt
    block_number,        // Felt
]
```

This call goes to the `KatanaTee` on-chain verifier contract.
There is **no** second call needed for saya (the sharding proxy call is sharding-specific).

---

## 7. Environment Variables

| Variable | Used by | Purpose |
|----------|---------|---------|
| `KATANA_RPC_URL` | `KatanaTeeClient::from_env()` | Katana TEE node RPC URL |
| `SP1_PRIVATE_KEY` | `ProverConfig::from_env()` | SP1 prover network auth |
| `SP1_RPC_URL` | `ProverConfig::from_env()` | SP1 prover RPC endpoint |

---

## 8. What saya Needs to Implement (TeeAttestor / TeeProver stubs)

### TeeAttestor (`saya/core/src/tee/attestor.rs`)

Replace the placeholder `fetch_attestation()` with:

1. Instantiate `KatanaTeeClient::new(&katana_rpc_url)`
2. Call `client.fetch_attestation()` → `Attested<BlockState>`
3. Call `client.fetch_quote_bytes()` → `Vec<u8>` (1184 bytes)
4. Wrap into `TeeAttestation { block_info, raw: quote_bytes }`

The `katana_rpc_url` is the rollup RPC — same URL used for the block ingestor.

### TeeProver (`saya/core/src/prover/tee/mod.rs`)

Replace the placeholder `prove()` with:

1. Deserialize `TeeAttestation` from trace bytes (or carry fields directly)
2. Build `SP1ProverConfig` from config
3. Build empty `StorageProofParams` (state roots from block state, empty keys/values)
4. Call `generate_proof_with_storage()` on `TeeAttestation`
5. Call `proof_to_calldata()` → `Vec<Felt>`
6. Return `TeeProof { block_info, data: proof_calldata_bytes }`

### OffchainTeeVerifier (`saya/core/src/tee/verifier.rs`)

In sharding-operator there is **no separate offchain verifier service** — verification
happens inline inside `generate_proof_with_storage()` (KDS cert fetch, cert chain
validation, state root cross-check). In saya's current architecture this stage is
a placeholder; it may be merged with `TeeProver` or kept as a pass-through.

---

## 9. What to Drop vs Keep for Saya

| Component | Sharding-operator | Saya |
|-----------|------------------|------|
| AMD SEV-SNP quote fetch | ✅ required | ✅ required |
| SP1 Groth16 proof generation | ✅ required | ✅ required |
| State root cross-check | ✅ required | ✅ required |
| `starknet_getStorageProof` RPC | ✅ required | ❌ drop |
| bonsai-trie Merkle verification | ✅ required | ❌ drop |
| StorageSlot keys/values | ✅ required | ❌ drop (empty arrays) |
| SCALE encoding of Merkle nodes | ✅ required | ❌ drop |
| Sharding proxy update call | ✅ required | ❌ drop |
| On-chain commitment nonce fetch | ✅ required (replay protection) | ⚠️ check if SP1 circuit requires it |
| Event inclusion proof | ❌ not yet implemented | ❌ skip |
| Proof caching by hash | optional | optional |

---

## 10. Key Source Files in sharding-operator

| File | Lines | Purpose |
|------|-------|---------|
| `src/tee/attestation.rs` | 396 | Core: quote parsing + SP1 proof generation |
| `src/tee/client.rs` | 79 | `KatanaTeeClient` — Katana RPC wrapper |
| `src/tee/sp1_adapter.rs` | 82 | Bridge from protocol traits to SP1 SDK |
| `src/shard/settlement.rs` | 212 | Pipeline orchestration |
| `src/shard/sp1.rs` | 332 | Storage proof params builder + proof caching |
| `src/shard/calls.rs` | 280 | On-chain call builders |
| `src/shard/verification.rs` | 229 | Block hash / state root cross-check |
| `src/protocol/pipeline.rs` | 143 | Core type definitions |
| `src/tee/storage_proof.rs` | 540 | (NOT NEEDED) Storage proof RPC + Merkle verification |
| `src/consts.rs` | 40 | Sepolia testnet defaults |
