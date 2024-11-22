use sqlx::{Pool, Row, Sqlite};

use super::sql_lite::SqliteDb;
use crate::errors::Error;

impl SqliteDb {
    // Function to check if the blocks table has the correct columns
    pub(crate) async fn check_columns(pool: &Pool<Sqlite>) -> Result<bool, Error> {
        let blocks_table = Self::check_blocks_table(pool).await?;
        let proof_table = Self::check_proof_table(pool).await?;
        Ok(blocks_table && proof_table)
    }
    pub(crate) async fn check_blocks_table(pool: &Pool<Sqlite>) -> Result<bool, Error> {
        let columns = sqlx::query("PRAGMA table_info(blocks);").fetch_all(pool).await?;
        // Check if the table has the expected columns: id, query_id, and status
        let mut has_id = false;
        let mut has_query_id_step1 = false;
        let mut has_query_id_step2 = false;
        let mut has_status = false;

        for column in columns {
            let name: String = column.get("name");
            match name.as_str() {
                "id" => has_id = true,
                "query_id_step1" => has_query_id_step1 = true,
                "query_id_step2" => has_query_id_step2 = true,
                "status" => has_status = true,
                _ => {}
            }
        }
        Ok(has_id && has_query_id_step1 && has_query_id_step2 && has_status)
    }
    pub(crate) async fn check_proof_table(pool: &Pool<Sqlite>) -> Result<bool, Error> {
        let columns = sqlx::query("PRAGMA table_info(proofs);").fetch_all(pool).await?;
        // Check if the table has the expected columns: id, block_number, and proof
        let mut has_id = false;
        let mut has_block_number = false;
        let mut has_pie_proof = false;
        let mut has_bridge_proof = false;

        for column in columns {
            let name: String = column.get("name");
            match name.as_str() {
                "id" => has_id = true,
                "block_number" => has_block_number = true,
                "pie_proof" => has_pie_proof = true,
                "bridge_proof" => has_bridge_proof = true,
                _ => {}
            }
        }
        Ok(has_id && has_block_number && has_pie_proof && has_bridge_proof)
    }
    // let table_exists =
    pub(crate) async fn check_table_exists(pool: &Pool<Sqlite>) -> Result<bool, Error> {
        let blocks_exist =
            sqlx::query("SELECT name FROM sqlite_master WHERE type='table' AND name='blocks';")
                .fetch_optional(pool)
                .await?
                .is_some();
        let proofs_exist =
            sqlx::query("SELECT name FROM sqlite_master WHERE type='table' AND name='proofs';")
                .fetch_optional(pool)
                .await?
                .is_some();
        Ok(blocks_exist && proofs_exist)
    }
}
