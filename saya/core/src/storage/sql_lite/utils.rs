use anyhow::Error;
use sqlx::{Pool, Row, Sqlite};

use super::SqliteDb;

impl SqliteDb {
    // Function to check if the blocks table has the correct columns
    pub(crate) async fn check_columns(pool: &Pool<Sqlite>) -> Result<bool, Error> {
        let blocks_table = Self::check_blocks_table(pool).await?;
        let proofs_table = Self::check_proof_table(pool).await?;
        let pies_table = Self::check_pies_table(pool).await?;
        let job_ids_table = Self::check_ids_table(pool).await?;
        let failed_blocks_table = Self::check_failed_blocks_table(pool).await?;
        Ok(blocks_table && proofs_table && pies_table && job_ids_table && failed_blocks_table)
    }

    pub(crate) async fn check_blocks_table(pool: &Pool<Sqlite>) -> Result<bool, Error> {
        let columns = sqlx::query("PRAGMA table_info(blocks);")
            .fetch_all(pool)
            .await?;
        // Check if the table has the expected columns: id and status
        let mut has_id = false;
        let mut has_status = false;

        for column in columns {
            let name: String = column.get("name");
            match name.as_str() {
                "block_id" => has_id = true,
                "status" => has_status = true,
                _ => {}
            }
        }
        Ok(has_id && has_status)
    }
    pub(crate) async fn check_proof_table(pool: &Pool<Sqlite>) -> Result<bool, Error> {
        let columns = sqlx::query("PRAGMA table_info(proofs);")
            .fetch_all(pool)
            .await?;
        // Check if the table has the expected columns: id, block_id, and proofs
        let mut has_id = false;
        let mut has_block_id = false;
        let mut has_snos_proof = false;
        let mut has_bridge_proof = false;

        for column in columns {
            let name: String = column.get("name");
            match name.as_str() {
                "id" => has_id = true,
                "block_id" => has_block_id = true,
                "snos_proof" => has_snos_proof = true,
                "bridge_proof" => has_bridge_proof = true,
                _ => {}
            }
        }
        Ok(has_id && has_block_id && has_snos_proof && has_bridge_proof)
    }
    pub(crate) async fn check_pies_table(pool: &Pool<Sqlite>) -> Result<bool, Error> {
        let columns = sqlx::query("PRAGMA table_info(pies);")
            .fetch_all(pool)
            .await?;
        // Check if the table has the expected columns: id, block_id, and proofs
        let mut has_id = false;
        let mut has_block_id = false;
        let mut has_snos_pie = false;
        let mut has_bridge_pie = false;

        for column in columns {
            let name: String = column.get("name");
            match name.as_str() {
                "id" => has_id = true,
                "block_id" => has_block_id = true,
                "snos_pie" => has_snos_pie = true,
                "bridge_pie" => has_bridge_pie = true,
                _ => {}
            }
        }
        Ok(has_id && has_block_id && has_snos_pie && has_bridge_pie)
    }

    pub(crate) async fn check_ids_table(pool: &Pool<Sqlite>) -> Result<bool, Error> {
        let columns = sqlx::query("PRAGMA table_info(job_ids);")
            .fetch_all(pool)
            .await?;
        // Check if the table has the expected columns: id, block_id, and proofs
        let mut has_id = false;
        let mut has_block_id = false;
        let mut has_snos_proof_query_id = false;
        let mut has_trace_gen_query_id = false;
        let mut has_bridge_proof_query_id = false;

        for column in columns {
            let name: String = column.get("name");
            match name.as_str() {
                "id" => has_id = true,
                "block_id" => has_block_id = true,
                "snos_proof_query_id" => has_snos_proof_query_id = true,
                "trace_gen_query_id" => has_trace_gen_query_id = true,
                "bridge_proof_query_id" => has_bridge_proof_query_id = true,
                _ => {}
            }
        }
        Ok(has_id
            && has_block_id
            && has_snos_proof_query_id
            && has_trace_gen_query_id
            && has_bridge_proof_query_id)
    }

    pub(crate) async fn check_failed_blocks_table(pool: &Pool<Sqlite>) -> Result<bool, Error> {
        let columns = sqlx::query("PRAGMA table_info(failed_blocks);")
            .fetch_all(pool)
            .await?;
        // Check if the table has the expected columns: id, block_id, and failure_reason
        let mut has_id = false;
        let mut has_block_id = false;
        let mut has_failure_reason = false;
        for column in columns {
            let name: String = column.get("name");
            match name.as_str() {
                "id" => has_id = true,
                "block_id" => has_block_id = true,
                "failure_reason" => has_failure_reason = true,
                _ => {}
            }
        }
        Ok(has_id && has_block_id && has_failure_reason)
    }
    pub(crate) async fn check_table_exists(pool: &Pool<Sqlite>) -> Result<bool, Error> {
        let expected_tables = vec!["blocks", "pies", "proofs", "job_ids", "failed_blocks"];
        for table in expected_tables {
            let exists =
                sqlx::query("SELECT name FROM sqlite_master WHERE type='table' AND name=?")
                    .bind(table)
                    .fetch_optional(pool)
                    .await?
                    .is_some();
            if !exists {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
