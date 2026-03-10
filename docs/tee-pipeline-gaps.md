# TEE Pipeline — Gap Analysis vs Persistent Flow

Tracks what is missing in the TEE pipeline compared to the persistent flow on `main`.

## Intentional Design Differences

These are **not gaps** — they are deliberate TEE mode decisions:

| Topic | Decision |
|---|---|
| Data Availability | Not needed in TEE mode (for now). TEE proof replaces DA-backed fraud proofs. |
| Fact registration | Happens on-chain at the settlement contract. No separate integrity verifier step needed. |
| State updates in settlement | TEE proof carries pre-computed state roots from inside the enclave. DB-stored state updates are not consumed by settlement. |

---

## Robustness Gaps (to implement after business logic is working)

### 1. Failed Block Recovery Queue

Persistent has `add_failed_block / get_failed_blocks` in DB; failed blocks are re-queued at ingestor startup and `db.remove_block()` is called on successful settlement.

TEE does `continue` at every error point (attestor, prover, settlement). Any single failure permanently drops the block with no recovery path.

---

### 2. Block Status Lifecycle Tracking

Persistent has a `BlockStatus` enum stored in DB with `db.set_status()` called at each transition:
```
Mined → ProofSubmitted → ProofGenerated → VerifiedProof → Settled  (+ Failed)
```

TEE state exists only in channels — non-queryable, non-resumable. Cannot answer "what stage is block N at?" and cannot resume partially-complete blocks on restart.

---

### 3. Proof Caching

Persistent stores proofs in DB (`add_proof / get_proof`). TEE passes `TeeProof` in-memory only. SP1 proof generation is expensive and always restarts from scratch after a crash.

---

### 4. Query ID Persistence

Persistent tracks remote prover query IDs (`add_query_id / get_query_id`) to poll for proof completion across restarts. TEE has no equivalent — if the prover is slow or the process crashes mid-generation there is no handle to resume.

---

### 5. Settlement DB Integration

Persistent settlement reads cached proofs from DB and calls `db.remove_block()` on success. TEE settlement has no DB — crash during settlement leaves the block in an unknown state on restart.

---

### 6. Attestor Retry Logic

`_poll_interval` is declared in `bin/saya/src/attestor.rs` but never used. On Katana RPC failure the entire batch is dropped with `continue`. Needs a retry loop that polls until attestation is available.

---

## Summary

| # | Gap | Severity |
|---|---|---|
| 1 | Failed block recovery | Critical |
| 2 | Block status tracking | Critical |
| 3 | Proof caching | High |
| 4 | Query ID persistence | High |
| 5 | Settlement DB integration | Critical |
| 6 | Attestor retry | Medium |

**Root cause:** The persistent pipeline is DB-driven — every stage checkpoints to SQLite and recovery re-reads it on restart. The TEE pipeline is purely channel-driven — no checkpointing, no resumability.

---

## Storage Layer Redesign

Before implementing the robustness gaps above, the storage layer itself needs to be cleaned up. The current `PersistantStorage` trait + `SqliteDb` impl has a number of issues that would make it wrong to extend as-is.

### Architecture decisions

**One `BlockStorage` trait + one `BlobStorage` trait, shared by both TEE and SNOS.**
Each saya instance operates on a single L3 chain with no cross-instance coordination, so SQLite + local filesystem is the right stack — no Postgres or object storage needed.

```rust
trait BlockStorage {
    // lifecycle
    async fn initialize_block(&self, block_number: u64) -> Result<()>;
    async fn remove_block(&self, block_number: u64) -> Result<()>;
    async fn set_status(&self, block_number: u64, status: BlockStatus) -> Result<()>;
    async fn get_status(&self, block_number: u64) -> Result<BlockStatus>;
    async fn get_first_unprocessed_block(&self) -> Result<u64>;
    async fn add_failed_block(&self, block_number: u64, reason: String) -> Result<()>;
    async fn get_failed_blocks(&self) -> Result<Vec<(u64, String)>>;
    async fn mark_failed_blocks_handled(&self, ids: &[u64]) -> Result<()>;
}

trait BlobStorage {
    async fn store_blob(&self, block_number: u64, kind: BlobKind, data: &[u8]) -> Result<()>;
    async fn get_blob(&self, block_number: u64, kind: BlobKind) -> Result<Vec<u8>>;
}

enum BlobKind {
    SnosPie,
    SnosProof,
    BridgeProof,
    TeeProof,
}

enum BlockStatus {
    Mined,
    // SNOS path
    SnosProofGenerated,
    BridgeProofGenerated,
    // TEE path
    Attested,
    TeeProofGenerated,
    // shared
    Settled,
    Failed,
}
```

`SqliteDb` implements `BlockStorage`. A `FilesystemBlobs` struct implements `BlobStorage` (stores files at `<base_dir>/<block>/<kind>.bin`).

### Issues in the current implementation to fix

| Issue | Fix |
|---|---|
| Manual `PRAGMA table_info` migration (~180 lines) | Replace with `sqlx::migrate!()` + versioned `.sql` files |
| `add_pie/add_proof/add_query_id` silently update `BlockStatus` as side effects | Remove; pipeline sets status explicitly via `set_status` |
| `set_status` takes `String` | Change to `set_status(BlockStatus)` — conversion belongs in the impl |
| `Step::Snos/Bridge` and `Query::SnosProof/BridgeProof/BridgeTrace` enums are SNOS-specific | Replace with `BlobKind` covering all pipelines |
| `add_pie/get_pie` + `add_proof/get_proof` are separate methods | Unify into `store_blob/get_blob` |
| PIE and proof blobs stored as SQLite BLOBs (can be hundreds of MB) | Move to filesystem; store only path or `(block, kind)` key in DB |
| `add_failed_block` deletes + re-inserts the block row (business logic in storage) | Strip to pure insert into `failed_blocks` |
| `failed_blocks` has no FK to `blocks` (orphan rows on block delete) | Add `REFERENCES blocks(block_id) ON DELETE CASCADE` |
| `handled = TRUE` rows accumulate forever | Delete on handle (or add `handled_at` timestamp for audit) |
| `max_connections(50)` on SQLite | Drop to 5; enable WAL mode |
| `u32` block numbers | `u64` |
