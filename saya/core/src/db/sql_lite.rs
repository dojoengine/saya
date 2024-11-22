use std::fs;
use std::path::Path;

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{query, Pool, Row, Sqlite};
use tracing::trace;

use super::Block;
use crate::db::{BlockStatus, SayaProvingDb};
use crate::errors::Error;

#[derive(Clone)]
pub struct SqliteDb {
    pub(crate) pool: Pool<Sqlite>,
}

impl SqliteDb {
    pub async fn new(path: &str) -> Result<Self, Error> {
        // Check if there is a database file at the path
        if !Path::new(path).try_exists()? {
            trace!("Database file not found. A new one will be created at: {}", path);
            fs::File::create(path)?;
        } else {
            trace!("Database file found at: {}", path);
        }

        let pool = SqlitePoolOptions::new().connect(&format!("sqlite:{}", path)).await?;

        let table_exists = Self::check_table_exists(&pool).await?;

        if !table_exists || !Self::check_columns(&pool).await? {
            trace!("Creating or updating the 'blocks' table...");
            Self::create_block_table(&pool).await?;
            Self::create_proof_table(&pool).await?;
        } else {
            trace!("Table 'blocks' with correct structure found.");
        }
        Ok(Self { pool })
    }

    // Function to create the blocks table with the correct schema
    pub async fn create_block_table(pool: &Pool<Sqlite>) -> Result<(), Error> {
        query(
            "CREATE TABLE blocks (
                id INTEGER PRIMARY KEY,
                query_id_step1 TEXT NOT NULL, 
                query_id_step2 TEXT,          
                status TEXT NOT NULL CHECK (status IN ('PIE_SUBMITTED', 'FAILED', \
             'PIE_PROOF_GENERATED', 'COMPLETED', 'BRIDGE_PROOF_SUBMITED'))
        );",
        )
        .execute(pool)
        .await?;
        Ok(())
    }
    pub async fn create_proof_table(pool: &Pool<Sqlite>) -> Result<(), Error> {
        query(
            "CREATE TABLE proofs (
                id INTEGER NOT NULL PRIMARY KEY,
                block_number INTEGER,
                pie_proof TEXT,
                bridge_proof TEXT,
                FOREIGN KEY (block_number) REFERENCES blocks(id)
        );",
        )
        .execute(pool)
        .await?;
        Ok(())
    }
    pub async fn list_blocks(&self) -> Result<Vec<Block>, Error> {
        let rows = query("SELECT id, query_id_step1, query_id_step2, status FROM blocks")
            .fetch_all(&self.pool)
            .await?;
        let mut result = Vec::new();
        for row in rows {
            let id = row.get("id");
            let query_id_step1 = row.get("query_id_step1");
            let query_id_step2 = row.get("query_id_step2");
            let status: &str = row.get("status");
            let status = BlockStatus::try_from(status)?;
            result.push(Block { id, query_id_step1, query_id_step2, status });
        }
        Ok(result)
    }
    pub async fn delete_proof(&self, block_id: u32) -> Result<(), Error> {
        query("DELETE FROM proofs WHERE block_number = ?1")
            .bind(block_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn delete_block(&self, block_id: u32) -> Result<(), Error> {
        query("DELETE FROM blocks WHERE id = ?1").bind(block_id).execute(&self.pool).await?;
        Ok(())
    }
}

impl SayaProvingDb for SqliteDb {
    async fn check_status(&self, block: u32) -> Result<Block, Error> {
        let rows =
            query("SELECT id, query_id_step1, query_id_step2, status FROM blocks WHERE id = ?1")
                .bind(block)
                .fetch_all(&self.pool)
                .await?;
        let result = &rows[0];
        let id = result.get("id");
        let query_id_step1 = result.get("query_id_step1");
        let query_id_step2 = result.get("query_id_step2");
        let status: &str = result.get("status");
        let status = BlockStatus::try_from(status)?;
        Ok(Block { id, query_id_step1, query_id_step2, status })
    }
    async fn insert_block(
        &self,
        block_id: u32,
        query_id: &str,
        status: BlockStatus,
    ) -> Result<(), Error> {
        query(
            "INSERT INTO blocks (id, query_id_step1, status) 
            VALUES (?1, ?2, ?3) 
            ON CONFLICT(id) DO UPDATE SET 
                query_id_step1 = excluded.query_id_step1,
                status = excluded.status",
        )
        .bind(block_id)
        .bind(query_id)
        .bind(status.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_block_status(&self, block_id: u32, status: BlockStatus) -> Result<(), Error> {
        query("UPDATE blocks SET status = ?1 WHERE id = ?2")
            .bind(status.as_str())
            .bind(block_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn update_block_query_id_for_bridge_proof(
        &self,
        block_id: u32,
        query_id: &str,
    ) -> Result<(), Error> {
        let mut transaction = self.pool.begin().await?;
        query("UPDATE blocks SET query_id_step2 = ?1 WHERE id = ?2")
            .bind(query_id)
            .bind(block_id)
            .execute(&mut *transaction)
            .await?;

        query("UPDATE blocks SET status = ?1 WHERE id = ?2")
            .bind(BlockStatus::BridgeProofSubmited.as_str())
            .bind(block_id)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(())
    }
    async fn list_blocks_with_status(&self, status: BlockStatus) -> Result<Vec<Block>, Error> {
        let rows = query(
            "SELECT id, query_id_step1, query_id_step2, status FROM blocks WHERE status = ?1",
        )
        .bind(status.as_str())
        .fetch_all(&self.pool)
        .await?;
        let mut result = Vec::new();
        for row in rows {
            let id = row.get("id");
            let query_id_step1 = row.get("query_id_step1");
            let query_id_step2 = row.get("query_id_step2");
            let status: &str = row.get("status");
            let status = BlockStatus::try_from(status)?;
            result.push(Block { id, query_id_step1, query_id_step2, status });
        }
        Ok(result)
    }
    async fn insert_pie_proof(&self, block_id: u32, proof: &str) -> Result<(), Error> {
        let mut transaction = self.pool.begin().await?;
        query("INSERT INTO proofs (block_number, pie_proof) VALUES (?1, ?2)")
            .bind(block_id)
            .bind(proof)
            .execute(&mut *transaction)
            .await?;
        query("UPDATE blocks SET status = ?1 WHERE id = ?2")
        .bind(BlockStatus::PieProofGenerated.as_str())
        .bind(block_id)
        .execute(&mut *transaction) // Use `&mut transaction` explicitly
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Insert the bridge proof into the database and update the status of the block to `COMPLETED`
    async fn insert_bridge_proof(&self, block_id: u32, proof: &str) -> Result<(), Error> {
        let mut transaction = self.pool.begin().await?;
        // Update the proof in the proofs table
        query("UPDATE proofs SET bridge_proof = ?2 WHERE block_number = ?1")
        .bind(block_id)
        .bind(proof)
        .execute(&mut *transaction) // Use `&mut transaction` explicitly
        .await?;

        // Update the status in the blocks table
        query("UPDATE blocks SET status = ?1 WHERE id = ?2")
        .bind(BlockStatus::Completed.as_str())
        .bind(block_id)
        .execute(&mut *transaction) // Use `&mut transaction` explicitly
        .await?;
        // Commit the transaction
        transaction.commit().await?;
        Ok(())
    }
    async fn get_pie_proof(&self, block_id: u32) -> Result<String, Error> {
        let row = query("SELECT pie_proof FROM proofs WHERE block_number = ?1")
            .bind(block_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get("pie_proof"))
    }
    async fn get_bridge_proof(&self, block_id: u32) -> Result<String, Error> {
        let row = query("SELECT bridge_proof FROM proofs WHERE block_number = ?1")
            .bind(block_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get("bridge_proof"))
    }
    async fn list_proof(&self) -> Result<Vec<String>, Error> {
        let rows = query("SELECT pie_proof FROM proofs").fetch_all(&self.pool).await?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.get("pie_proof"));
        }
        Ok(result)
    }
}
