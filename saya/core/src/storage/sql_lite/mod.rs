use std::fs;
use std::path::Path;

use anyhow::Error;
use log::trace;
use sqlx::query;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Pool;
use sqlx::Sqlite;

mod utils;
mod storage;
#[derive(Clone)]
pub struct SqliteDb {
    pub(crate) pool: Pool<Sqlite>,
}

impl SqliteDb {
    pub async fn new(path: &str) -> Result<Self,Error> {
        // Check if there is a database file at the path
        if !Path::new(path).try_exists()? {
            trace!(
                "Database file not found. A new one will be created at: {}",
                path
            );
            fs::File::create(path)?;
        } else {
            trace!("Database file found at: {}", path);
        }

        let pool = SqlitePoolOptions::new()
            .connect(&format!("sqlite:{}", path))
            .await?;

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
    //each block should have those fields: 
    // id (block_number), snos_pie, snos_proof_query, bridge_pie_query, bridge_proof, status
    pub async fn create_block_table(pool: &Pool<Sqlite>) -> Result<(), Error> {
        query(
            "CREATE TABLE blocks (
                id INTEGER PRIMARY KEY,
                snos_pie BLOB,
                query_id_step1 TEXT, 
                layout_bridge_pie BLOB,
                query_id_step2 TEXT,          
                status TEXT NOT NULL CHECK (status IN ('MINED','PIE_SUBMITTED', 'FAILED', \
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
                pie_proof BLOB,
                bridge_proof BLOB,
                FOREIGN KEY (block_number) REFERENCES blocks(id)
        );",
        )
        .execute(pool)
        .await?;
        Ok(())
    }
    pub async fn delete_proof(&self, block_id: u32) -> Result<(), Error> {
        query("DELETE FROM proofs WHERE block_number = ?1")
            .bind(block_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn delete_block(&self, block_id: u32) -> Result<(), Error> {
        query("DELETE FROM blocks WHERE id = ?1")
            .bind(block_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
