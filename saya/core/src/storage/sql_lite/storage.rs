use super::SqliteDb;
use crate::storage::{BlockStatus, Query};
use crate::storage::{PersistantStorage, Step};
use sqlx::query;
use sqlx::Row;

impl PersistantStorage for SqliteDb {
    async fn add_pie(
        &self,
        block_number: u32,
        pie: Vec<u8>,
        step: Step,
    ) -> Result<(), anyhow::Error> {
        let new_status = match step {
            Step::Bridge => "bridge_pie_generated",
            Step::Snos => "snos_pie_generated",
        };
        let mut tx = self.pool.begin().await?;

        query(
            "INSERT OR IGNORE INTO pies (block_id, snos_pie, bridge_pie) VALUES (?, NULL, NULL);",
        )
        .bind(block_number)
        .execute(&mut *tx)
        .await?;

        match step {
            Step::Bridge => {
                query("UPDATE pies SET bridge_pie = ? WHERE block_id = ?;")
                    .bind(pie)
                    .bind(block_number)
                    .execute(&mut *tx)
                    .await?;
            }
            Step::Snos => {
                query("UPDATE pies SET snos_pie = ? WHERE block_id = ?;")
                    .bind(pie)
                    .bind(block_number)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        query("UPDATE blocks SET status = ? WHERE block_id = ?;")
            .bind(new_status)
            .bind(block_number)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn get_pie(&self, block_number: u32, step: Step) -> Result<Vec<u8>, anyhow::Error> {
        let column = match step {
            Step::Snos => "snos_pie",
            Step::Bridge => "bridge_pie",
        };

        let row = query(&format!("SELECT {} FROM pies WHERE id = ?1", column))
            .bind(block_number)
            .fetch_one(&self.pool)
            .await?;
        let pie: Vec<u8> = row.try_get(0)?;
        if pie.is_empty() {
            return Err(anyhow::anyhow!("Pie not found"));
        }
        Ok(pie)
    }

    async fn add_proof(
        &self,
        block_number: u32,
        proof: Vec<u8>,
        step: Step,
    ) -> Result<(), anyhow::Error> {
        let new_status = match step {
            Step::Bridge => "bridge_proof_generated",
            Step::Snos => "snos_proof_generated",
        };
        let mut tx = self.pool.begin().await?;
        // Ensure a row exists in proofs before updating
        query("INSERT OR IGNORE INTO proofs (block_id, snos_proof, bridge_proof) VALUES (?, NULL, NULL);")
            .bind(block_number)
            .execute(&mut *tx)
            .await?;

        let column = match step {
            Step::Snos => "snos_proof",
            Step::Bridge => "bridge_proof",
        };

        query(&format!(
            "UPDATE proofs SET {} = ? WHERE block_id = ?",
            column
        ))
        .bind(proof)
        .bind(block_number)
        .execute(&mut *tx)
        .await?;
        query("UPDATE blocks SET status = ? WHERE block_id = ?;")
            .bind(new_status)
            .bind(block_number)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn get_proof(&self, block_number: u32, step: Step) -> Result<Vec<u8>, anyhow::Error> {
        let column = match step {
            Step::Snos => "snos_proof",
            Step::Bridge => "bridge_proof",
        };

        let row = query(&format!(
            "SELECT {} FROM proofs WHERE block_id = ?1",
            column
        ))
        .bind(block_number)
        .fetch_one(&self.pool)
        .await?;

        let proof: Vec<u8> = row.try_get(0)?;
        if proof.is_empty() {
            return Err(anyhow::anyhow!("Proof not found"));
        }
        Ok(proof)
    }

    async fn add_query_id(
        &self,
        block_number: u32,
        query_id: String,
        query_type: Query,
    ) -> Result<(), anyhow::Error> {
        let new_status = match query_type {
            Query::BridgeProof => "bridge_proof_submitted",
            Query::BridgeTrace => "bridge_pie_submitted",
            Query::SnosProof => "snos_proof_submitted",
        };

        let mut tx = self.pool.begin().await?;
        query(
            "INSERT OR IGNORE INTO job_ids (block_id, snos_proof_query_id, trace_gen_query_id, bridge_proof_query_id) VALUES (?, NULL, NULL, NULL);",
        )
        .bind(block_number)
        .execute(&mut *tx)
        .await?;

        let column = match query_type {
            Query::BridgeProof => "bridge_proof_query_id",
            Query::BridgeTrace => "trace_gen_query_id",
            Query::SnosProof => "snos_proof_query_id",
        };
        query(&format!(
            "UPDATE job_ids SET {} = ? WHERE block_id = ?",
            column
        ))
        .bind(query_id)
        .bind(block_number)
        .execute(&mut *tx)
        .await?;
        query("UPDATE blocks SET status = ? WHERE block_id = ?;")
            .bind(new_status)
            .bind(block_number)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn get_query_id(
        &self,
        block_number: u32,
        query_type: Query,
    ) -> Result<String, anyhow::Error> {
        let column = match query_type {
            Query::BridgeProof => "bridge_proof_query_id",
            Query::BridgeTrace => "trace_gen_query_id",
            Query::SnosProof => "snos_proof_query_id",
        };

        let row = query(&format!(
            "SELECT {} FROM job_ids WHERE block_id = ?1",
            column
        ))
        .bind(block_number)
        .fetch_one(&self.pool)
        .await?;

        let query_id: String = row.try_get(0)?;
        if query_id.is_empty() {
            return Err(anyhow::anyhow!("Query ID not found"));
        }
        Ok(query_id)
    }

    async fn set_status(&self, block_number: u32, status: String) -> Result<(), anyhow::Error> {
        query("UPDATE blocks SET status = ?1 WHERE block_id = ?2")
            .bind(status)
            .bind(block_number)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_status(&self, block_number: u32) -> Result<BlockStatus, anyhow::Error> {
        let row = query("SELECT status FROM blocks WHERE block_id = ?1")
            .bind(block_number)
            .fetch_one(&self.pool)
            .await?;

        let status: String = row.try_get(0)?;
        let status = BlockStatus::from(status.as_str());
        Ok(status)
    }
    async fn initialize_block(&self, block_number: u32) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;

        query("INSERT OR IGNORE INTO blocks (block_id, status) VALUES (?1, 'mined')")
            .bind(block_number)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn remove_block(&self, block_number: u32) -> anyhow::Result<()> {
        query("DELETE FROM blocks WHERE block_id = ?1")
            .bind(block_number)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};
    async fn get_tmp() -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_micros();
        format!("tmp-{}.db", now)
    }
    async fn cleanup_db(path: &str) {
        if Path::new(path).exists() {
            std::fs::remove_file(path).unwrap();
        }
    }
    #[tokio::test]
    async fn test_initialize_and_remove_block() {
        let db_path = get_tmp().await;

        let db = SqliteDb::new(&db_path).await.unwrap();

        // Initialize block
        db.initialize_block(1).await.unwrap();
        let status = db.get_status(1).await.unwrap();
        assert_eq!(status, BlockStatus::Mined);

        // Remove block
        db.remove_block(1).await.unwrap();
        let result = db.get_status(1).await;
        assert!(
            result.is_err(),
            "Block should be removed, but status was found"
        );
        cleanup_db(&db_path).await;
    }

    #[tokio::test]
    async fn test_add_and_get_pie_for_multiple_blocks() {
        let db_path = get_tmp().await;

        let db = SqliteDb::new(&db_path).await.unwrap();

        // Initialize multiple blocks
        db.initialize_block(1).await.unwrap();
        db.initialize_block(2).await.unwrap();

        let pie1 = vec![1, 2, 3, 4, 5];
        let pie2 = vec![6, 7, 8, 9, 10];

        db.add_pie(1, pie1.clone(), Step::Snos).await.unwrap();
        db.add_pie(2, pie2.clone(), Step::Bridge).await.unwrap();

        let result1 = db.get_pie(1, Step::Snos).await.unwrap();
        let result2 = db.get_pie(2, Step::Bridge).await.unwrap();

        assert_eq!(result1, pie1);
        assert_eq!(result2, pie2);
        cleanup_db(&db_path).await;
    }

    #[tokio::test]
    async fn test_add_pie_does_not_overwrite_other_pie() {
        let db_path = get_tmp().await;

        let db = SqliteDb::new(&db_path).await.unwrap();

        db.initialize_block(1).await.unwrap();
        let snos_pie = vec![1, 2, 3];
        let bridge_pie = vec![4, 5, 6];

        db.add_pie(1, snos_pie.clone(), Step::Snos).await.unwrap();
        db.add_pie(1, bridge_pie.clone(), Step::Bridge)
            .await
            .unwrap();

        let result_snos = db.get_pie(1, Step::Snos).await.unwrap();
        let result_bridge = db.get_pie(1, Step::Bridge).await.unwrap();

        assert_eq!(result_snos, snos_pie);
        assert_eq!(result_bridge, bridge_pie);
        cleanup_db(&db_path).await;
    }

    #[tokio::test]
    async fn test_add_and_get_proof_for_multiple_blocks() {
        let db_path = get_tmp().await;

        let db = SqliteDb::new(&db_path).await.unwrap();

        db.initialize_block(1).await.unwrap();
        db.initialize_block(2).await.unwrap();

        let proof1 = vec![10, 20, 30, 40];
        let proof2 = vec![50, 60, 70, 80];

        db.add_proof(1, proof1.clone(), Step::Snos).await.unwrap();
        db.add_proof(2, proof2.clone(), Step::Bridge).await.unwrap();

        let result1 = db.get_proof(1, Step::Snos).await.unwrap();
        let result2 = db.get_proof(2, Step::Bridge).await.unwrap();

        assert_eq!(result1, proof1);
        assert_eq!(result2, proof2);
        cleanup_db(&db_path).await;
    }

    #[tokio::test]
    async fn test_get_pie_returns_error_for_missing_block() {
        let db_path = get_tmp().await;

        let db = SqliteDb::new(&db_path).await.unwrap();

        let result = db.get_pie(99, Step::Snos).await;
        assert!(
            result.is_err(),
            "Expected error when getting pie for non-existent block"
        );
        cleanup_db(&db_path).await;
    }

    #[tokio::test]
    async fn test_get_proof_returns_error_for_missing_block() {
        let db_path = get_tmp().await;

        let db = SqliteDb::new(&db_path).await.unwrap();

        let result = db.get_proof(99, Step::Snos).await;
        assert!(
            result.is_err(),
            "Expected error when getting proof for non-existent block"
        );
        cleanup_db(&db_path).await;
    }

    #[tokio::test]
    async fn test_add_and_get_query_id_for_multiple_blocks() {
        let db_path = get_tmp().await;

        let db = SqliteDb::new(&db_path).await.unwrap();

        db.initialize_block(1).await.unwrap();
        db.initialize_block(2).await.unwrap();

        let query_id_1 = "query_1".to_string();
        let query_id_2 = "query_2".to_string();

        db.add_query_id(1, query_id_1.clone(), Query::BridgeProof)
            .await
            .unwrap();
        db.add_query_id(2, query_id_2.clone(), Query::SnosProof)
            .await
            .unwrap();

        let result_1 = db.get_query_id(1, Query::BridgeProof).await.unwrap();
        let result_2 = db.get_query_id(2, Query::SnosProof).await.unwrap();

        assert_eq!(result_1, query_id_1);
        assert_eq!(result_2, query_id_2);
        cleanup_db(&db_path).await;
    }

    #[tokio::test]
    async fn test_query_id_does_not_overwrite_other_query_ids() {
        let db_path = get_tmp().await;

        println!("&db_path: {:?}", db_path);
        let db = SqliteDb::new(&db_path).await.unwrap();

        db.initialize_block(1).await.unwrap();

        let snos_query_id = "snos_123".to_string();
        let bridge_query_id = "bridge_456".to_string();

        db.add_query_id(1, snos_query_id.clone(), Query::SnosProof)
            .await
            .unwrap();
        db.add_query_id(1, bridge_query_id.clone(), Query::BridgeProof)
            .await
            .unwrap();

        let result_snos = db.get_query_id(1, Query::SnosProof).await.unwrap();
        let result_bridge = db.get_query_id(1, Query::BridgeProof).await.unwrap();

        assert_eq!(result_snos, snos_query_id);
        assert_eq!(result_bridge, bridge_query_id);
        cleanup_db(&db_path).await;
    }

    #[tokio::test]
    async fn test_set_and_get_status() {
        let db_path = get_tmp().await;

        let db = SqliteDb::new(&db_path).await.unwrap();

        db.initialize_block(1).await.unwrap();

        db.set_status(1, "snos_proof_submitted".to_string())
            .await
            .unwrap();
        let status = db.get_status(1).await.unwrap();
        assert_eq!(status, BlockStatus::SnosProofSubmitted);

        db.set_status(1, "bridge_proof_generated".to_string())
            .await
            .unwrap();
        let status = db.get_status(1).await.unwrap();
        assert_eq!(status, BlockStatus::BridgeProofGenerated);
        cleanup_db(&db_path).await;
    }

    #[tokio::test]
    async fn test_get_status_returns_error_for_missing_block() {
        let db_path = get_tmp().await;

        let db = SqliteDb::new(&db_path).await.unwrap();

        let result = db.get_status(99).await;
        assert!(
            result.is_err(),
            "Expected error when getting status for non-existent block"
        );
        cleanup_db(&db_path).await;
    }

    #[tokio::test]
    async fn test_remove_block_deletes_pies_and_proofs() {
        let db_path = get_tmp().await;

        let db = SqliteDb::new(&db_path).await.unwrap();

        db.initialize_block(1).await.unwrap();

        let pie = vec![1, 2, 3];
        let proof = vec![4, 5, 6];

        db.add_pie(1, pie.clone(), Step::Snos).await.unwrap();
        db.add_proof(1, proof.clone(), Step::Snos).await.unwrap();

        db.remove_block(1).await.unwrap();

        let pie_result = db.get_pie(1, Step::Snos).await;
        let proof_result = db.get_proof(1, Step::Snos).await;

        assert!(
            pie_result.is_err(),
            "Expected error when getting pie for deleted block"
        );
        assert!(
            proof_result.is_err(),
            "Expected error when getting proof for deleted block"
        );
        cleanup_db(&db_path).await;
    }
}
