# Saya Architecture

Saya is a proving and settlement layer for Starknet-based rollups (Katana).
It ingests blocks, generates SNOS proofs, publishes them to a DA layer, and
optionally settles on a base-layer contract (Piltover).

---

## Modes at a Glance

| Mode            | Prover             | DA Layer        | Settlement         | Use case                       |
|-----------------|--------------------|-----------------|--------------------|--------------------------------|
| **Sovereign**   | Atlantic (SNOS)    | Celestia        | None (self-hosted) | Fully autonomous rollup        |
| **Persistent**  | Atlantic (SNOS + Bridge) | Celestia / Noop | Piltover (L2/L1) | L3 settling to L2 or L1       |
| **Persistent TEE** | TEE attestation | None            | Piltover (L2/L1)   | Hardware-backed, fast settle   |

---

## Component Flow Diagram

```
╔══════════════════════════════════════════════════════════════════════════════════════════╗
║                        SAYA – Starknet Rollup Proving System                            ║
╚══════════════════════════════════════════════════════════════════════════════════════════╝

                              ┌──────────────────────────┐
                              │    Katana Rollup Node    │
                              │   (Starknet JSON-RPC)    │
                              └──────────────┬───────────┘
                                             │ blocks & state diffs
                              ┌──────────────▼───────────┐
                              │   PollingBlockIngestor   │
                              │  BlockInfo { Mined }     │
                              └──┬──────────┬────────┬───┘
                                 │          │        │
              ┌──────────────────┘          │        └──────────────────────┐
              │                             │                               │
              ▼                             ▼                               ▼

┌─────────────────────────┐ ┌─────────────────────────┐ ┌─────────────────────────┐
│     SOVEREIGN MODE      │ │    PERSISTENT MODE      │ │   PERSISTENT TEE MODE   │
│  Prove + DA, no settle  │ │ Prove + DA + Settlement │ │  TEE attestation only   │
└────────────┬────────────┘ └────────────┬────────────┘ └────────────┬────────────┘
             │                           │                            │
             ▼                           ▼                            ▼
┌─────────────────────────┐ ┌─────────────────────────┐ ┌─────────────────────────┐
│   SnosPieGenerator      │ │   SnosPieGenerator      │ │      BlockOrderer       │
│  (generate_pie crate)   │ │  (generate_pie crate)   │ │  Buffer & sort by #     │
│  → Cairo PIE artifact   │ │  → Cairo PIE artifact   │ │  BlockInfo { Mined }    │
└────────────┬────────────┘ └────────────┬────────────┘ └────────────┬────────────┘
             │                           │                            │
             ▼                           ▼                            │ (no prover)
┌─────────────────────────┐ ┌─────────────────────────┐              │
│   AtlanticSnosProver    │ │   AtlanticSnosProver    │              ▼
│  Poll Atlantic HTTP API │ │  Poll Atlantic HTTP API │ ┌─────────────────────────┐
│  → SnosProof<StarkProof>│ │  → SnosProof<String>    │ │   PiltoverSettlement    │
└────────────┬────────────┘ └────────────┬────────────┘ │  Piltover::update_state │
             │                           │               │  TEE attestation proof  │
             ▼                           ▼               └────────────┬────────────┘
┌─────────────────────────┐ ┌─────────────────────────┐              │
│      BlockOrderer       │ │  AtlanticLayoutBridge   │              ▼
│  Buffer & sort by #     │ │  Verifies SNOS proof    │ ┌─────────────────────────┐
│  → sequential order     │ │  → bridge proof ready   │ │    SettlementCursor     │
└────────────┬────────────┘ └────────────┬────────────┘ │  { block_number,        │
             │                           │               │    transaction_hash }   │
             ▼                           ▼               └─────────────────────────┘
┌─────────────────────────┐ ┌─────────────────────────┐
│  CelestiaDA Backend     │ │      BlockOrderer       │
│  Serialize SovereignPkt │ │  Buffer & sort by #     │
│  Submit blob to Celestia│ │  → sequential order     │
│  → DA pointer           │ └────────────┬────────────┘
│  { height, commit, ns } │              │
└────────────┬────────────┘              ▼
             │              ┌─────────────────────────┐
             ▼              │  CelestiaDA / Noop      │
┌─────────────────────────┐ │  Serialize PersistentPkt│
│  InMemory / SQLiteDb    │ │  → DA pointer or None   │
│  Store chain head       │ └────────────┬────────────┘
│  with DA pointer        │              │
│  ChainHead::Block { }   │              ▼
└─────────────────────────┘ ┌─────────────────────────┐
                            │   PiltoverSettlement    │
                            │  update_state() call    │
                            │  Integrity fact verify  │
                            │  Poll tx inclusion      │
                            └────────────┬────────────┘
                                         │
                                         ▼
                            ┌─────────────────────────┐
                            │    SettlementCursor     │
                            │  { block_number,        │
                            │    transaction_hash }   │
                            └─────────────────────────┘
```

> **Note on BlockOrderer placement:** SNOS proving and Layout Bridge proving are
> async HTTP calls to Atlantic that can complete out of order. `BlockOrderer` is
> placed **after all provers** (outermost `PipelineChain`) so the DA layer always
> receives blocks in ascending sequence.

---

## Component Descriptions

### Block Ingestor (`saya/core/src/block_ingestor/`)

- **`PollingBlockIngestor`** — polls the Katana RPC for new blocks, fetches state
  diffs, stores them in the storage backend, and emits `BlockInfo { status: Mined }`
  into the pipeline channel.

### Proving Pipeline (`saya/core/src/prover/`)

Stage order inside each pipeline (outermost = last, closest to DA):

| Stage | Input | Output | Modes | Description |
|-------|-------|--------|-------|-------------|
| `SnosPieGenerator` | `BlockInfo { Mined }` | `BlockInfo { SnosPieGenerated }` | Sovereign, Persistent | Generates Cairo PIE from state diff using `generate_pie` |
| `AtlanticSnosProver` | `BlockInfo { SnosPieGenerated }` | `SnosProof<StarkProof\|String>` | Sovereign, Persistent | Submits PIE to Atlantic, polls for proof |
| `AtlanticLayoutBridgeProver` | `SnosProof<String>` | `BlockInfo { BridgeProofGenerated }` | Persistent only | Verifies SNOS proof via layout bridge program |
| `BlockOrderer<T>` | Out-of-order items | In-order items | Sovereign, Persistent, TEE | BTreeMap buffer; re-emits ascending by block number |
| `PipelineChain` | upstream | downstream | — | Composes two `PipelineStage` impls with a bridge channel |

### Data Availability (`saya/core/src/data_availability/`)

| Backend | Packet type | Description |
|---------|-------------|-------------|
| `CelestiaDataAvailabilityBackend` | `SovereignPacket` / `PersistentPacket` | CBOR-serializes proof, submits blob, returns `DataAvailabilityPointer { height, commitment, namespace }` |
| `NoopDataAvailabilityBackend` | — | Pass-through; no blob published; pointer = `None` |

### Settlement (`saya/core/src/settlement/`)

- **`PiltoverSettlementBackend`** — receives `DataAvailabilityCursor`, fetches SNOS
  output, optionally registers facts with the Integrity verifier, then calls
  `Piltover::update_state()` and polls for inclusion. Emits `SettlementCursor`.

### Storage (`saya/core/src/storage/`)

| Backend | Used by | Description |
|---------|---------|-------------|
| `SqliteDb` | Persistent, Sovereign | Persistent storage: PIEs, proofs, state updates, block status |
| `InMemoryStorageBackend` | Sovereign (light) | Ephemeral in-memory chain head + DA pointer |

### Orchestrators (`saya/core/src/orchestrator/`)

| Orchestrator | File | Pipeline composition |
|---|---|---|
| `SovereignOrchestrator` | `orchestrator/sovereign.rs` | Ingestor → `SnosPieGen → SnosProver → BlockOrderer` → CelestiaDA → Storage |
| `PersistentOrchestrator` | `orchestrator/persistent.rs` | Ingestor → `SnosPieGen → SnosProver → LayoutBridge → BlockOrderer` → DA → PiltoverSettlement |
| `PersistentTeeOrchestrator` | `orchestrator/persistent_tee.rs` | Ingestor → BlockOrderer → Adapter → PiltoverSettlement |

All components implement the `Daemon` trait with graceful shutdown via
`CancellationToken` + `FinishHandle`.

---

## CLI Entry Points (`bin/saya/src/`)

```
saya sovereign         start --starknet-rpc <URL> --atlantic-key <KEY>
                              --celestia-rpc <URL> --celestia-token <TOKEN>

saya persistent        start --rollup-rpc <URL> --settlement-rpc <URL>
                              --settlement-piltover-address <ADDR>
                              --layout-bridge-program <PATH>
                              --atlantic-key <KEY>

saya persistent-tee    start --rollup-rpc <URL> --settlement-rpc <URL>
                              --settlement-piltover-address <ADDR>

saya core-contract     deploy ...
saya celestia          ...
```
