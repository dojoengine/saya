use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::Error;
use log::trace;
use sqlx::query;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Pool;
use sqlx::Sqlite;

mod storage;
mod utils;
#[derive(Clone)]
pub struct SqliteDb {
    pub(crate) pool: Pool<Sqlite>,
}

impl SqliteDb {
    pub async fn new(path: &str) -> Result<Self, Error> {
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
            .acquire_timeout(Duration::from_secs(10))
            .max_connections(50)
            .connect(&format!("sqlite:{}", path))
            .await?;

        let table_exists = Self::check_table_exists(&pool).await?;

        if !table_exists || !Self::check_columns(&pool).await? {
            trace!("Creating or updating the 'blocks' table...");
            Self::create_block_table(&pool).await?;
            Self::create_proof_table(&pool).await?;
            Self::create_pies_table(&pool).await?;
            Self::create_job_id_table(&pool).await?;
            Self::create_failed_blocks_table(&pool).await?;
        } else {
            trace!("Table 'blocks' with correct structure found.");
        }
        Ok(Self { pool })
    }

    pub async fn create_block_table(pool: &Pool<Sqlite>) -> Result<(), Error> {
        query(
            r#"
            CREATE TABLE IF NOT EXISTS blocks (
                block_id INTEGER PRIMARY KEY,
                status TEXT NOT NULL CHECK (
                    status IN (
                        'mined',
                        'snos_pie_generated',
                        'snos_proof_submitted',
                        'snos_proof_generated',
                        'bridge_pie_submitted',
                        'bridge_pie_generated',
                        'bridge_proof_submitted',
                        'bridge_proof_generated',
                        'verified_proof',
                        'settled',
                        'failed'
                    )
                )
            );
            "#,
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn create_pies_table(pool: &Pool<Sqlite>) -> Result<(), Error> {
        query(
            r#"
            CREATE TABLE IF NOT EXISTS pies (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              block_id INTEGER NOT NULL REFERENCES blocks(block_id) ON DELETE CASCADE,
              snos_pie BLOB,
              bridge_pie BLOB
            );
            "#,
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn create_proof_table(pool: &Pool<Sqlite>) -> Result<(), Error> {
        query(
            r#"CREATE TABLE IF NOT EXISTS proofs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                block_id INTEGER NOT NULL REFERENCES blocks(block_id) ON DELETE CASCADE,
                snos_proof BLOB,
                bridge_proof BLOB
        );"#,
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn create_job_id_table(pool: &Pool<Sqlite>) -> Result<(), Error> {
        query(
            r#"CREATE TABLE IF NOT EXISTS job_ids (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            block_id INTEGER NOT NULL REFERENCES blocks(block_id) ON DELETE CASCADE,
            snos_proof_query_id TEXT,
            trace_gen_query_id TEXT,
            bridge_proof_query_id TEXT
            );
            "#,
        )
        .execute(pool)
        .await?;
        Ok(())
    }
    pub async fn create_failed_blocks_table(pool: &Pool<Sqlite>) -> Result<(), Error> {
        query(
            r#"
            CREATE TABLE IF NOT EXISTS failed_blocks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                block_id INTEGER NOT NULL REFERENCES blocks(block_id),
                failure_reason TEXT NOT NULL
            );
            "#,
        )
        .execute(pool)
        .await?;
        Ok(())
    }
}
