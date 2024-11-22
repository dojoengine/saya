#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use crate::db::sql_lite::SqliteDb;
    use crate::db::{BlockStatus, SayaProvingDb};

    async fn setup_db() -> SqliteDb {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("Failed to create database pool in memory");

        let db = SqliteDb { pool };
        SqliteDb::create_block_table(&db.pool).await.expect("Failed to create blocks table");
        SqliteDb::create_proof_table(&db.pool).await.expect("Failed to create proofs table");

        db
    }
    #[tokio::test]
    async fn test_insert_block() {
        let db = setup_db().await;

        db.insert_block(1, "query_1", BlockStatus::PieSubmitted)
            .await
            .expect("Failed to insert block");

        let block = db.check_status(1).await.expect("Failed to check block status");
        assert_eq!(block.id, 1);
        assert_eq!(block.query_id_step1, "query_1");
        assert_eq!(block.status, BlockStatus::PieSubmitted);
    }
    #[tokio::test]
    async fn test_update_block_status() {
        let db = setup_db().await;

        db.insert_block(2, "query_2", BlockStatus::PieSubmitted)
            .await
            .expect("Failed to insert block");

        db.update_block_status(2, BlockStatus::PieProofGenerated)
            .await
            .expect("Failed to update block status");

        let block = db.check_status(2).await.expect("Failed to check block status");
        assert_eq!(block.status, BlockStatus::PieProofGenerated);
    }
    #[tokio::test]
    async fn test_update_query_id_step2() {
        let db = setup_db().await;

        db.insert_block(3, "query_3", BlockStatus::PieSubmitted)
            .await
            .expect("Failed to insert block");

        db.update_block_query_id_for_bridge_proof(3, "query_3_step2")
            .await
            .expect("Failed to update query_id_step2");

        let block = db.check_status(3).await.expect("Failed to check block status");
        assert_eq!(block.query_id_step2, "query_3_step2");
    }
    #[tokio::test]
    async fn test_list_blocks_with_status() {
        let db = setup_db().await;

        db.insert_block(4, "query_4", BlockStatus::PieSubmitted)
            .await
            .expect("Failed to insert block");
        db.insert_block(5, "query_5", BlockStatus::PieProofGenerated)
            .await
            .expect("Failed to insert block");

        let submitted_blocks = db
            .list_blocks_with_status(BlockStatus::PieSubmitted)
            .await
            .expect("Failed to list blocks with status");

        assert_eq!(submitted_blocks.len(), 1);
        assert_eq!(submitted_blocks[0].id, 4);
    }
    #[tokio::test]
    async fn test_insert_pie_proof() {
        let db = setup_db().await;

        db.insert_block(6, "query_6", BlockStatus::PieSubmitted)
            .await
            .expect("Failed to insert block");

        db.insert_pie_proof(6, "pie_proof_data").await.expect("Failed to insert pie proof");

        let pie_proof = db.get_pie_proof(6).await.expect("Failed to get pie proof");
        assert_eq!(pie_proof, "pie_proof_data");
    }
    #[tokio::test]
    async fn test_insert_bridge_proof() {
        let db = setup_db().await;

        db.insert_block(7, "query_7", BlockStatus::PieProofGenerated)
            .await
            .expect("Failed to insert block");
        db.insert_pie_proof(7, "pie_proof_data").await.expect("Failed to insert pie proof");

        db.insert_bridge_proof(7, "bridge_proof_data")
            .await
            .expect("Failed to insert bridge proof");

        let bridge_proof = db.get_bridge_proof(7).await.expect("Failed to get bridge proof");
        assert_eq!(bridge_proof, "bridge_proof_data");
    }
    #[tokio::test]
    async fn test_get_pie_proof() {
        let db = setup_db().await;

        db.insert_block(8, "query_8", BlockStatus::PieSubmitted)
            .await
            .expect("Failed to insert block");
        db.insert_pie_proof(8, "pie_proof_data").await.expect("Failed to insert pie proof");

        let pie_proof = db.get_pie_proof(8).await.expect("Failed to get pie proof");
        assert_eq!(pie_proof, "pie_proof_data");
    }
    #[tokio::test]
    async fn test_get_bridge_proof() {
        let db = setup_db().await;

        db.insert_block(9, "query_9", BlockStatus::PieProofGenerated)
            .await
            .expect("Failed to insert block");
        db.insert_pie_proof(9, "pie_proof_data").await.expect("Failed to insert pie proof");
        db.insert_bridge_proof(9, "bridge_proof_data")
            .await
            .expect("Failed to insert bridge proof");

        let bridge_proof = db.get_bridge_proof(9).await.expect("Failed to get bridge proof");
        assert_eq!(bridge_proof, "bridge_proof_data");
    }
    #[tokio::test]
    async fn test_list_proof() {
        let db = setup_db().await;

        db.insert_block(10, "query_10", BlockStatus::PieSubmitted)
            .await
            .expect("Failed to insert block");
        db.insert_pie_proof(10, "pie_proof_data_1").await.expect("Failed to insert pie proof");

        db.insert_block(11, "query_11", BlockStatus::PieSubmitted)
            .await
            .expect("Failed to insert block");
        db.insert_pie_proof(11, "pie_proof_data_2").await.expect("Failed to insert pie proof");

        let proofs = db.list_proof().await.expect("Failed to list proofs");
        assert_eq!(proofs.len(), 2);
        assert!(proofs.contains(&"pie_proof_data_1".to_string()));
        assert!(proofs.contains(&"pie_proof_data_2".to_string()));
    }
}
