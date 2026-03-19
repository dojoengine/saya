# Saya Integration Plan — Piltover TEE Messaging

## Context

Piltover has been updated (branch `feat/tee-persistent`) to support messages in the TEE input path.
Two files changed:
- `src/input/tee_input.cairo` — `TEEInput` gains 4 new fields
- `src/input/component.cairo` — `validate_input` checks `messages_commitment`; `get_messages` returns real messages
- `piltover/src/bindgen.rs` — regenerated; `TEEInput` Rust struct has the 4 new fields

Current HEAD in piltover: `3567340` (bindgen regeneration commit).
The revision to use in saya: **the latest commit on `feat/tee-persistent`** — run
`git -C /home/mateusz/dev/starknet/piltover rev-parse HEAD` to get it.

## Steps

### 1. Update piltover dependency in saya

In `/home/mateusz/dev/starknet/saya/Cargo.toml`, the workspace-level piltover dep is:
```toml
piltover = { package = "piltover", git = "https://github.com/cartridge-gg/piltover.git", rev = "67e65b8..." }
```

**Option A — point at the git branch (when pushed):**
```toml
piltover = { package = "piltover", git = "https://github.com/cartridge-gg/piltover.git", rev = "<new-HEAD>" }
```

**Option B — use local path while developing:**
```toml
piltover = { package = "piltover", path = "/home/mateusz/dev/starknet/piltover/piltover" }
```

### 2. Update the compiled contract in saya

The compiled Appchain contract JSON lives at:
  `/home/mateusz/dev/starknet/saya/contracts/core_contract.json`

Replace it with the freshly compiled artifact from piltover:
  `/home/mateusz/dev/starknet/piltover/target/dev/piltover_Appchain.contract_class.json`

(Check exact filename with `ls /home/mateusz/dev/starknet/piltover/target/dev/*.json`)

### 3. Wire up TEEInput fields in Saya's TEE settlement backend

Saya currently has no TEE-specific settlement backend — the `persistent_tee` mode reuses
`PiltoverSettlementBackend` with `skip_fact_registration(true)` and submits
`PiltoverInput::LayoutBridgeOutputNoDa`. This needs to change to submit `PiltoverInput::TeeInput`.

The TEE settlement backend needs to:
1. Call `katana_tee_client::KatanaRpcClient::fetch_attestation(prev_block, block)` to get `TeeQuoteResponse`
2. Run `AmdAttestationProver::prove()` to get the SP1 proof (`OnchainProof → Vec<Felt>`)
3. Build `piltover::TEEInput`:
   ```rust
   TEEInput {
       sp1_proof:            /* proof felts */,
       prev_state_root:      quote.prev_state_root,
       state_root:           quote.state_root,
       prev_block_hash:      quote.prev_block_hash,
       block_hash:           quote.block_hash,
       prev_block_number:    quote.prev_block_number,
       block_number:         quote.block_number,
       messages_commitment:  quote.messages_commitment,
       messages_to_starknet: /* convert quote.l2_to_l1_messages → Vec<MessageToStarknet> */,
       messages_to_appchain: /* convert quote.l1_to_l2_messages → Vec<MessageToAppchain> */,
       l1_to_l2_msg_hashes:  /* Vec<Felt> — the keccak256 hashes from l1_to_l2 receipts */,
   }
   ```
4. Call `piltover_contract.update_state(PiltoverInput::TeeInput(tee_input))`

### 4. Message type conversions

`katana_tee_client::TeeL2ToL1Message` → `piltover::MessageToStarknet`:
```rust
MessageToStarknet {
    from_address: msg.from_address,
    to_address:   msg.to_address,
    payload:      msg.payload,
}
```

`katana_tee_client::TeeL1ToL2Message` → `piltover::MessageToAppchain`:
```rust
MessageToAppchain {
    from_address: msg.from_address,
    to_address:   msg.to_address,
    nonce:        msg.nonce,
    selector:     msg.selector,
    payload:      msg.payload,
}
```

`l1_to_l2_msg_hashes`: these are the keccak256 hashes (as felts) from Katana's L1Handler receipts.
The katana-tee client does NOT currently expose these separately — they are embedded in
`messages_commitment` but not returned as a list. Check whether `TeeQuoteResponse` in katana
(not katana-tee) returns them, or whether saya needs to recompute them from `l1_to_l2_messages`.
**This may require a small addition to katana's `TeeQuoteResponse`.**

### Notes

- `piltover::MessageToStarknet` and `piltover::MessageToAppchain` types are in `piltover::bindgen`
  (auto-generated). They use `starknet::core::types::Felt`, not `starknet_types_core::felt::Felt`
  — check if a conversion is needed.
- The existing `PiltoverSettlementBackend` in `saya/core/src/settlement/piltover.rs` handles
  STARK proofs only. Either extend it with a TEE mode flag, or create a separate
  `TeeSettlementBackend` that wraps the katana-tee client + proof generation.
