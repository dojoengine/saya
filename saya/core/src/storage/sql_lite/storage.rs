use super::SqliteDb;
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
        let column = match step {
            Step::Snos => "snos_pie",
            Step::Bridge => "layout_bridge_pie",
        };

        query(&format!("UPDATE blocks SET {} = ?1 WHERE id = ?2", column))
            .bind(pie)
            .bind(block_number)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_pie(&self, block_number: u32, step: Step) -> Result<Vec<u8>, anyhow::Error> {
        let column = match step {
            Step::Snos => "snos_pie",
            Step::Bridge => "layout_bridge_pie",
        };

        let row = query(&format!("SELECT {} FROM blocks WHERE id = ?1", column))
            .bind(block_number)
            .fetch_one(&self.pool)
            .await?;
        let pie: Vec<u8> = row.try_get(0)?;
        Ok(pie)
    }

    async fn add_proof(
        &self,
        block_number: u32,
        proof: Vec<u8>,
        step: Step,
    ) -> Result<(), anyhow::Error> {
        let column = match step {
            Step::Snos => "pie_proof",
            Step::Bridge => "bridge_proof",
        };

        query(&format!(
            "UPDATE proofs SET {} = ?1 WHERE block_number = ?2",
            column
        ))
        .bind(proof)
        .bind(block_number)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_proof(&self, block_number: u32, step: Step) -> Result<Vec<u8>, anyhow::Error> {
        let column = match step {
            Step::Snos => "pie_proof",
            Step::Bridge => "bridge_proof",
        };

        let row = query(&format!(
            "SELECT {} FROM proofs WHERE block_number = ?1",
            column
        ))
        .bind(block_number)
        .fetch_one(&self.pool)
        .await?;

        let proof: Vec<u8> = row.try_get(0)?;
        Ok(proof)
    }

    async fn add_query_id(
        &self,
        block_number: u32,
        query_id: Vec<u8>,
        step: Step,
    ) -> Result<(), anyhow::Error> {
        let column = match step {
            Step::Snos => "query_id_step1",
            Step::Bridge => "query_id_step2",
        };

        query(&format!("UPDATE blocks SET {} = ?1 WHERE id = ?2", column))
            .bind(query_id)
            .bind(block_number)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_query_id(&self, block_number: u32, step: Step) -> Result<Vec<u8>, anyhow::Error> {
        let column = match step {
            Step::Snos => "query_id_step1",
            Step::Bridge => "query_id_step2",
        };

        let row = query(&format!("SELECT {} FROM blocks WHERE id = ?1", column))
            .bind(block_number)
            .fetch_one(&self.pool)
            .await?;

        let query_id: Vec<u8> = row.try_get(0)?;
        Ok(query_id)
    }

    async fn set_status(&self, block_number: u32, status: String) -> Result<(), anyhow::Error> {
        query("UPDATE blocks SET status = ?1 WHERE id = ?2")
            .bind(status)
            .bind(block_number)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_status(&self, block_number: u32) -> Result<String, anyhow::Error> {
        let row = query("SELECT status FROM blocks WHERE id = ?1")
            .bind(block_number)
            .fetch_one(&self.pool)
            .await?;

        let status: String = row.try_get(0)?;
        Ok(status)
    }
    async fn initialize_block(&self, block_number: u32) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;

        // Insert into blocks table
        sqlx::query("INSERT INTO blocks (id, status) VALUES (?1, 'MINED')")
            .bind(block_number)
            .execute(&mut *tx)
            .await?;

        // Insert into proofs table, initializing with NULL proofs
        sqlx::query(
            "INSERT INTO proofs (block_number, pie_proof, bridge_proof) VALUES (?1, NULL, NULL)",
        )
        .bind(block_number)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn remove_block(&self, block_number: u32) -> anyhow::Result<()> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_add_and_get_pie() {
        let db = SqliteDb::new("test.db").await.unwrap();
        let pie_data = vec![1, 2, 3];

        db.initialize_block(1).await.unwrap();
        db.add_pie(1, pie_data.clone(), Step::Bridge).await.unwrap();
        let retrieved_pie = db.get_pie(1, Step::Bridge).await.unwrap();

        assert_eq!(retrieved_pie, pie_data);
    }

    #[tokio::test]
    async fn test_add_and_get_proof() {
        let db = SqliteDb::new("test.db").await.unwrap();
        let proof_data = vec![4, 5, 6];

        db.initialize_block(2).await.unwrap();
        db.add_proof(2, proof_data.clone(), Step::Snos)
            .await
            .unwrap();
        let retrieved_proof = db.get_proof(2, Step::Snos).await.unwrap();

        assert_eq!(retrieved_proof, proof_data);
    }

    #[tokio::test]
    async fn test_add_and_get_query_id() {
        let db = SqliteDb::new("test.db").await.unwrap();
        let query_id_data = vec![7, 8, 9];

        db.initialize_block(3).await.unwrap();
        db.add_query_id(3, query_id_data.clone(), Step::Bridge)
            .await
            .unwrap();
        let retrieved_query_id = db.get_query_id(3, Step::Bridge).await.unwrap();

        assert_eq!(retrieved_query_id, query_id_data);
    }

    #[tokio::test]
    async fn test_set_and_get_status() {
        let db = SqliteDb::new("test.db").await.unwrap();
        let status = "PIE_SUBMITTED".to_string();

        db.initialize_block(4).await.unwrap();
        db.set_status(4, status.clone()).await.unwrap();
        let retrieved_status = db.get_status(4).await.unwrap();

        assert_eq!(retrieved_status, status);
    }
}
