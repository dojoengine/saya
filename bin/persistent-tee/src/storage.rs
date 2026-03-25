//! TEE persistence backend using SQLite.

use anyhow::{anyhow, Result};
use log::{debug, trace};
use saya_core::tee::storage::{
    BatchId, IncompleteBatch, StoredAttestation, TeeBatchStatus, TeeStorage,
};
use saya_core::storage::SqliteDb;
use sqlx::{query, Row};

/// TEE-specific SQLite database wrapper.
#[derive(Clone)]
pub struct TeeDb {
    db: SqliteDb,
}

impl std::fmt::Debug for TeeDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeeDb").finish_non_exhaustive()
    }
}

impl TeeDb {
    /// Create a new TEE database connection, initializing schema if needed.
    pub async fn new(path: &str) -> Result<Self> {
        let db = SqliteDb::new(path).await?;
        Self::create_tee_tables(&db).await?;
        Ok(TeeDb { db })
    }

    /// Initialize TEE-specific tables.
    async fn create_tee_tables(db: &SqliteDb) -> Result<()> {
        // tee_batches: metadata for each batch
        query(
            r#"
            CREATE TABLE IF NOT EXISTS tee_batches (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                first_block     INTEGER NOT NULL,
                last_block      INTEGER NOT NULL,
                status          TEXT NOT NULL CHECK (status IN (
                                    'pending_attestation',
                                    'attested',
                                    'proved',
                                    'settlement_pending',
                                    'settled',
                                    'failed'
                                )),
                retry_count     INTEGER NOT NULL DEFAULT 0,
                created_at      INTEGER NOT NULL
            );
            "#,
        )
        .execute(&db.pool)
        .await?;

        // tee_attestations: stored attestation data
        query(
            r#"
            CREATE TABLE IF NOT EXISTS tee_attestations (
                batch_id        INTEGER PRIMARY KEY REFERENCES tee_batches(id) ON DELETE CASCADE,
                quote           TEXT NOT NULL,
                prev_state_root TEXT NOT NULL,
                state_root      TEXT NOT NULL,
                prev_block_hash TEXT NOT NULL,
                block_hash      TEXT NOT NULL,
                prev_block_number TEXT NOT NULL,
                block_number    TEXT NOT NULL,
                messages_commitment TEXT NOT NULL,
                l2_to_l1_messages TEXT NOT NULL,
                l1_to_l2_messages TEXT NOT NULL
            );
            "#,
        )
        .execute(&db.pool)
        .await?;

        // tee_proofs: stored SP1 Groth16 proofs (JSON-encoded)
        query(
            r#"
            CREATE TABLE IF NOT EXISTS tee_proofs (
                batch_id        INTEGER PRIMARY KEY REFERENCES tee_batches(id) ON DELETE CASCADE,
                proof_bytes     BLOB NOT NULL
            );
            "#,
        )
        .execute(&db.pool)
        .await?;

        // tee_settlement_txs: settlement transaction tracking
        query(
            r#"
            CREATE TABLE IF NOT EXISTS tee_settlement_txs (
                batch_id        INTEGER PRIMARY KEY REFERENCES tee_batches(id) ON DELETE CASCADE,
                tx_hash         TEXT NOT NULL
            );
            "#,
        )
        .execute(&db.pool)
        .await?;

        trace!("TEE tables initialized");
        Ok(())
    }
}

impl TeeStorage for TeeDb {
    fn create_batch(
        &self,
        first_block: u64,
        last_block: u64,
    ) -> impl std::future::Future<Output = Result<BatchId>> + Send {
        let db_clone = self.db.clone();
        async move {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs() as i64;

            let result = query(
                r#"
                INSERT INTO tee_batches (first_block, last_block, status, retry_count, created_at)
                VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(first_block as i64)
            .bind(last_block as i64)
            .bind("pending_attestation")
            .bind(0i32)
            .bind(now)
            .execute(&db_clone.pool)
            .await?;

            let batch_id = result.last_insert_rowid();
            debug!(batch_id, first_block, last_block; "Created batch");
            Ok(batch_id)
        }
    }

    fn set_batch_status(
        &self,
        id: BatchId,
        status: TeeBatchStatus,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let db_clone = self.db.clone();
        async move {
            let status_str = status.to_string();
            query("UPDATE tee_batches SET status = ? WHERE id = ?")
                .bind(&status_str)
                .bind(id)
                .execute(&db_clone.pool)
                .await?;
            debug!(batch_id = id, status = status.to_string().as_str(); "Updated batch status");
            Ok(())
        }
    }

    fn increment_retry_count(
        &self,
        id: BatchId,
    ) -> impl std::future::Future<Output = Result<u32>> + Send {
        let db_clone = self.db.clone();
        async move {
            let result = query("UPDATE tee_batches SET retry_count = retry_count + 1 WHERE id = ?")
                .bind(id)
                .execute(&db_clone.pool)
                .await?;

            if result.rows_affected() == 0 {
                return Err(anyhow!("Batch {} not found", id));
            }

            let row = query("SELECT retry_count FROM tee_batches WHERE id = ?")
                .bind(id)
                .fetch_one(&db_clone.pool)
                .await?;

            let retry_count: i32 = row.get("retry_count");
            Ok(retry_count as u32)
        }
    }

    fn save_attestation(
        &self,
        id: BatchId,
        stored_data: &StoredAttestation,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let db_clone = self.db.clone();
        let stored_data = stored_data.clone();
        async move {
            query(
                r#"
                INSERT OR REPLACE INTO tee_attestations (
                    batch_id, quote, prev_state_root, state_root, prev_block_hash, block_hash,
                    prev_block_number, block_number, messages_commitment, l2_to_l1_messages, l1_to_l2_messages
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(id)
            .bind(&stored_data.quote)
            .bind(&stored_data.prev_state_root)
            .bind(&stored_data.state_root)
            .bind(&stored_data.prev_block_hash)
            .bind(&stored_data.block_hash)
            .bind(&stored_data.prev_block_number)
            .bind(&stored_data.block_number)
            .bind(&stored_data.messages_commitment)
            .bind(&stored_data.l2_to_l1_messages)
            .bind(&stored_data.l1_to_l2_messages)
            .execute(&db_clone.pool)
            .await?;

            debug!(batch_id = id; "Saved attestation");
            Ok(())
        }
    }

    fn load_attestation(
        &self,
        id: BatchId,
    ) -> impl std::future::Future<Output = Result<Option<StoredAttestation>>> + Send {
        let db_clone = self.db.clone();
        async move {
            let row = query(
                r#"
                SELECT quote, prev_state_root, state_root, prev_block_hash, block_hash,
                       prev_block_number, block_number, messages_commitment, l2_to_l1_messages, l1_to_l2_messages
                FROM tee_attestations WHERE batch_id = ?
                "#,
            )
            .bind(id)
            .fetch_optional(&db_clone.pool)
            .await?;

            Ok(row.map(|r| StoredAttestation {
                quote: r.get("quote"),
                prev_state_root: r.get("prev_state_root"),
                state_root: r.get("state_root"),
                prev_block_hash: r.get("prev_block_hash"),
                block_hash: r.get("block_hash"),
                prev_block_number: r.get("prev_block_number"),
                block_number: r.get("block_number"),
                messages_commitment: r.get("messages_commitment"),
                l2_to_l1_messages: r.get("l2_to_l1_messages"),
                l1_to_l2_messages: r.get("l1_to_l2_messages"),
            }))
        }
    }

    fn save_proof(
        &self,
        id: BatchId,
        proof_bytes: &[u8],
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let db_clone = self.db.clone();
        let proof_bytes = proof_bytes.to_vec();
        async move {
            query("INSERT OR REPLACE INTO tee_proofs (batch_id, proof_bytes) VALUES (?, ?)")
                .bind(id)
                .bind(&proof_bytes)
                .execute(&db_clone.pool)
                .await?;

            debug!(batch_id = id, proof_size = proof_bytes.len(); "Saved proof");
            Ok(())
        }
    }

    fn load_proof(
        &self,
        id: BatchId,
    ) -> impl std::future::Future<Output = Result<Option<Vec<u8>>>> + Send {
        let db_clone = self.db.clone();
        async move {
            let row = query("SELECT proof_bytes FROM tee_proofs WHERE batch_id = ?")
                .bind(id)
                .fetch_optional(&db_clone.pool)
                .await?;

            Ok(row.map(|r| r.get("proof_bytes")))
        }
    }

    fn save_settlement_tx(
        &self,
        id: BatchId,
        tx_hash: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let db_clone = self.db.clone();
        let tx_hash = tx_hash.to_string();
        async move {
            query("INSERT OR REPLACE INTO tee_settlement_txs (batch_id, tx_hash) VALUES (?, ?)")
                .bind(id)
                .bind(&tx_hash)
                .execute(&db_clone.pool)
                .await?;

            debug!(batch_id = id, tx_hash = tx_hash.as_str(); "Saved settlement tx");
            Ok(())
        }
    }

    fn get_settlement_tx(
        &self,
        id: BatchId,
    ) -> impl std::future::Future<Output = Result<Option<String>>> + Send {
        let db_clone = self.db.clone();
        async move {
            let row = query("SELECT tx_hash FROM tee_settlement_txs WHERE batch_id = ?")
                .bind(id)
                .fetch_optional(&db_clone.pool)
                .await?;

            Ok(row.map(|r| r.get("tx_hash")))
        }
    }

    fn confirm_settlement_tx(
        &self,
        id: BatchId,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let db_clone = self.db.clone();
        async move {
            query("DELETE FROM tee_settlement_txs WHERE batch_id = ?")
                .bind(id)
                .execute(&db_clone.pool)
                .await?;

            debug!(batch_id = id; "Confirmed settlement tx");
            Ok(())
        }
    }

    fn get_incomplete_batches(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<IncompleteBatch>>> + Send {
        let db_clone = self.db.clone();
        async move {
            let rows = query(
                r#"
                SELECT b.id, b.first_block, b.last_block, b.status, b.retry_count
                FROM tee_batches b
                WHERE b.status NOT IN ('settled', 'failed')
                ORDER BY b.created_at ASC
                "#,
            )
            .fetch_all(&db_clone.pool)
            .await?;

            let mut incomplete = Vec::new();
            for row in rows {
                let batch_id: i64 = row.get("id");
                let first_block: i64 = row.get("first_block");
                let last_block: i64 = row.get("last_block");
                let status_str: String = row.get("status");
                let retry_count: i32 = row.get("retry_count");

                let status: TeeBatchStatus = status_str.parse()
                    .map_err(|e: String| anyhow::anyhow!(e))?;

                // Load related data
                let attestation = {
                    let row = query(
                        r#"
                        SELECT quote, prev_state_root, state_root, prev_block_hash, block_hash,
                               prev_block_number, block_number, messages_commitment, l2_to_l1_messages, l1_to_l2_messages
                        FROM tee_attestations WHERE batch_id = ?
                        "#,
                    )
                    .bind(batch_id)
                    .fetch_optional(&db_clone.pool)
                    .await?;

                    row.map(|r| StoredAttestation {
                        quote: r.get("quote"),
                        prev_state_root: r.get("prev_state_root"),
                        state_root: r.get("state_root"),
                        prev_block_hash: r.get("prev_block_hash"),
                        block_hash: r.get("block_hash"),
                        prev_block_number: r.get("prev_block_number"),
                        block_number: r.get("block_number"),
                        messages_commitment: r.get("messages_commitment"),
                        l2_to_l1_messages: r.get("l2_to_l1_messages"),
                        l1_to_l2_messages: r.get("l1_to_l2_messages"),
                    })
                };

                let proof = {
                    let row = query("SELECT proof_bytes FROM tee_proofs WHERE batch_id = ?")
                        .bind(batch_id)
                        .fetch_optional(&db_clone.pool)
                        .await?;

                    row.map(|r| r.get("proof_bytes"))
                };

                let settlement_tx_hash = {
                    let row = query("SELECT tx_hash FROM tee_settlement_txs WHERE batch_id = ?")
                        .bind(batch_id)
                        .fetch_optional(&db_clone.pool)
                        .await?;

                    row.map(|r| r.get("tx_hash"))
                };

                incomplete.push(IncompleteBatch {
                    batch_id,
                    first_block: first_block as u64,
                    last_block: last_block as u64,
                    status,
                    attestation,
                    proof,
                    settlement_tx_hash,
                    retry_count: retry_count as u32,
                });
            }

            debug!(count = incomplete.len(); "Loaded incomplete batches");
            Ok(incomplete)
        }
    }

    fn get_last_settled_block(
        &self,
    ) -> impl std::future::Future<Output = Result<Option<u64>>> + Send {
        let db_clone = self.db.clone();
        async move {
            let row = query(
                r#"
                SELECT MAX(last_block) as last_block
                FROM tee_batches
                WHERE status = 'settled'
                "#,
            )
            .fetch_one(&db_clone.pool)
            .await?;

            let last_block: Option<i64> = row.get("last_block");
            Ok(last_block.map(|b| b as u64))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_core::tee::storage::{StoredAttestation, TeeBatchStatus, TeeStorage};

    async fn in_memory_db() -> TeeDb {
        TeeDb::new(":memory:").await.unwrap()
    }

    fn sample_attestation() -> StoredAttestation {
        StoredAttestation {
            quote: "quote".into(),
            prev_state_root: "0x0".into(),
            state_root: "0x1".into(),
            prev_block_hash: "0x0".into(),
            block_hash: "0x1".into(),
            prev_block_number: "0x0".into(),
            block_number: "0x1".into(),
            messages_commitment: "0x0".into(),
            l2_to_l1_messages: "[]".into(),
            l1_to_l2_messages: "[]".into(),
        }
    }

    // ── batch lifecycle ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_batch_returns_id() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();
        assert!(id > 0);
        let batches = db.get_incomplete_batches().await.unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].batch_id, id);
        assert_eq!(batches[0].first_block, 1);
        assert_eq!(batches[0].last_block, 10);
        assert_eq!(batches[0].status, TeeBatchStatus::PendingAttestation);
    }

    #[tokio::test]
    async fn settled_batch_absent_from_incomplete() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();
        db.set_batch_status(id, TeeBatchStatus::Settled).await.unwrap();
        assert!(db.get_incomplete_batches().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_batch_absent_from_incomplete() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();
        db.set_batch_status(id, TeeBatchStatus::Failed).await.unwrap();
        assert!(db.get_incomplete_batches().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn status_transitions_visible_in_incomplete() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();

        for status in [
            TeeBatchStatus::Attested,
            TeeBatchStatus::Proved,
            TeeBatchStatus::SettlementPending,
        ] {
            db.set_batch_status(id, status).await.unwrap();
            let batches = db.get_incomplete_batches().await.unwrap();
            assert_eq!(batches[0].status, status, "expected {status:?}");
        }
    }

    #[tokio::test]
    async fn increment_retry_count_is_monotone() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();
        assert_eq!(db.increment_retry_count(id).await.unwrap(), 1);
        assert_eq!(db.increment_retry_count(id).await.unwrap(), 2);
        assert_eq!(db.increment_retry_count(id).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn get_last_settled_block_none_when_empty() {
        let db = in_memory_db().await;
        assert!(db.get_last_settled_block().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_last_settled_block_tracks_max() {
        let db = in_memory_db().await;
        let id1 = db.create_batch(1, 10).await.unwrap();
        let id2 = db.create_batch(11, 20).await.unwrap();

        db.set_batch_status(id1, TeeBatchStatus::Settled).await.unwrap();
        assert_eq!(db.get_last_settled_block().await.unwrap(), Some(10));

        db.set_batch_status(id2, TeeBatchStatus::Settled).await.unwrap();
        assert_eq!(db.get_last_settled_block().await.unwrap(), Some(20));
    }

    // ── attestation ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn attestation_round_trip() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();
        let att = sample_attestation();
        db.save_attestation(id, &att).await.unwrap();

        let loaded = db.load_attestation(id).await.unwrap().unwrap();
        assert_eq!(loaded.quote, att.quote);
        assert_eq!(loaded.state_root, att.state_root);
        assert_eq!(loaded.l2_to_l1_messages, att.l2_to_l1_messages);
    }

    #[tokio::test]
    async fn load_attestation_returns_none_when_absent() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();
        assert!(db.load_attestation(id).await.unwrap().is_none());
    }

    /// Scratch retry: saving an attestation a second time must not error and
    /// the second write must win (INSERT OR REPLACE semantics).
    #[tokio::test]
    async fn save_attestation_idempotent_on_scratch_retry() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();

        db.save_attestation(id, &sample_attestation()).await.unwrap();

        let mut second = sample_attestation();
        second.state_root = "0x2".into();
        db.save_attestation(id, &second).await.unwrap(); // must not fail

        let loaded = db.load_attestation(id).await.unwrap().unwrap();
        assert_eq!(loaded.state_root, "0x2");
    }

    // ── proof ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn proof_round_trip() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();
        db.save_proof(id, &[1, 2, 3]).await.unwrap();

        assert_eq!(db.load_proof(id).await.unwrap().unwrap(), vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn load_proof_returns_none_when_absent() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();
        assert!(db.load_proof(id).await.unwrap().is_none());
    }

    /// Scratch retry: saving proof a second time must not error and second write wins.
    #[tokio::test]
    async fn save_proof_idempotent_on_scratch_retry() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();

        db.save_proof(id, &[1, 2, 3]).await.unwrap();
        db.save_proof(id, &[4, 5, 6]).await.unwrap(); // must not fail

        assert_eq!(db.load_proof(id).await.unwrap().unwrap(), vec![4, 5, 6]);
    }

    // ── settlement tx ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn settlement_tx_round_trip() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();

        db.save_settlement_tx(id, "0xabc").await.unwrap();
        assert_eq!(
            db.get_settlement_tx(id).await.unwrap().unwrap(),
            "0xabc"
        );
    }

    #[tokio::test]
    async fn confirm_settlement_tx_removes_row() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();

        db.save_settlement_tx(id, "0xabc").await.unwrap();
        db.confirm_settlement_tx(id).await.unwrap();
        assert!(db.get_settlement_tx(id).await.unwrap().is_none());
    }

    /// Scratch retry for settlement: clear tx then re-submit; the second INSERT
    /// must succeed and the new hash must be stored.
    #[tokio::test]
    async fn settlement_scratch_retry_after_confirm() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();

        db.save_settlement_tx(id, "0xfirst").await.unwrap();
        db.confirm_settlement_tx(id).await.unwrap(); // clear before scratch retry
        db.save_settlement_tx(id, "0xsecond").await.unwrap(); // must not fail

        assert_eq!(
            db.get_settlement_tx(id).await.unwrap().unwrap(),
            "0xsecond"
        );
    }

    // ── incomplete batch recovery ────────────────────────────────────────────

    #[tokio::test]
    async fn incomplete_batches_include_stored_attestation_and_proof() {
        let db = in_memory_db().await;
        let id = db.create_batch(1, 10).await.unwrap();

        db.save_attestation(id, &sample_attestation()).await.unwrap();
        db.set_batch_status(id, TeeBatchStatus::Attested).await.unwrap();

        let batches = db.get_incomplete_batches().await.unwrap();
        assert!(batches[0].attestation.is_some());
        assert!(batches[0].proof.is_none());

        db.save_proof(id, &[1, 2, 3]).await.unwrap();
        db.set_batch_status(id, TeeBatchStatus::Proved).await.unwrap();

        let batches = db.get_incomplete_batches().await.unwrap();
        assert!(batches[0].attestation.is_some());
        assert!(batches[0].proof.is_some());
    }

    #[tokio::test]
    async fn incomplete_batches_ordered_by_creation_time() {
        let db = in_memory_db().await;
        let id1 = db.create_batch(1, 10).await.unwrap();
        let id2 = db.create_batch(11, 20).await.unwrap();

        let batches = db.get_incomplete_batches().await.unwrap();
        assert_eq!(batches[0].batch_id, id1);
        assert_eq!(batches[1].batch_id, id2);
    }
}
