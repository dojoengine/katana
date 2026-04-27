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

        let row = query(&format!("SELECT {} FROM pies WHERE block_id = ?1", column))
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

    async fn get_first_db_block(&self) -> Result<u32, anyhow::Error> {
        let row = query("SELECT MIN(block_id) FROM blocks")
            .fetch_one(&self.pool)
            .await?;
        let first_block: u32 = row.try_get(0)?;
        Ok(first_block)
    }

    async fn add_failed_block(
        &self,
        block_number: u32,
        failure_reason: String,
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;

        // Remove the faulty block from blocks table
        query("DELETE FROM blocks WHERE block_id = ?1")
            .bind(block_number)
            .execute(&mut *tx)
            .await?;
        // Initialize the block
        query("INSERT OR IGNORE INTO blocks (block_id, status) VALUES (?1, 'mined')")
            .bind(block_number)
            .execute(&mut *tx)
            .await?;
        // Add the block to failed_blocks table
        query("INSERT INTO failed_blocks (block_id, failure_reason) VALUES (?1, ?2)")
            .bind(block_number)
            .bind(failure_reason)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn get_failed_blocks(&self) -> anyhow::Result<Vec<(u32, String)>> {
        let mut failed_blocks = Vec::new();
        let rows =
            query("SELECT block_id, failure_reason FROM failed_blocks WHERE handled = FALSE")
                .fetch_all(&self.pool)
                .await?;
        for row in rows {
            let block_id: u32 = row.try_get(0)?;
            let failure_reason: String = row.try_get(1)?;
            failed_blocks.push((block_id, failure_reason));
        }
        Ok(failed_blocks)
    }

    async fn mark_failed_blocks_as_handled(&self, block_ids: &[u32]) -> anyhow::Result<()> {
        if block_ids.is_empty() {
            return Ok(()); // Nothing to update
        }

        let mut query = String::from("UPDATE failed_blocks SET handled = TRUE WHERE block_id IN (");
        query.push_str(&block_ids.iter().map(|_| "?").collect::<Vec<_>>().join(","));
        query.push(')');

        let mut sql_query = sqlx::query(&query);
        for id in block_ids {
            sql_query = sql_query.bind(id);
        }

        sql_query.execute(&self.pool).await?;
        Ok(())
    }

    async fn add_state_update(
        &self,
        block_number: u32,
        state_update: starknet::core::types::StateUpdate,
    ) -> anyhow::Result<()> {
        let serialized = serde_json::to_vec(&state_update)?;
        let mut tx = self.pool.begin().await?;
        query("INSERT OR REPLACE INTO state_updates (block_id, state_update) VALUES (?, ?);")
            .bind(block_number)
            .bind(serialized)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn get_state_update(
        &self,
        block_number: u32,
    ) -> anyhow::Result<starknet::core::types::StateUpdate> {
        let row = query("SELECT state_update FROM state_updates WHERE block_id = ?1")
            .bind(block_number)
            .fetch_one(&self.pool)
            .await?;
        let serialized: Vec<u8> = row.try_get(0)?;
        let state_update: starknet::core::types::StateUpdate = serde_json::from_slice(&serialized)?;
        Ok(state_update)
    }
}

#[cfg(test)]
mod tests {
    use starknet::core::types::StateDiff;
    use starknet_types_core::felt::Felt;

    use crate::storage::sql_lite::IN_MEMORY_DB;

    use super::*;

    #[tokio::test]
    async fn test_initialize_and_remove_block() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

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
    }

    #[tokio::test]
    async fn test_add_and_get_pie_for_multiple_blocks() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

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
    }

    #[tokio::test]
    async fn test_add_pie_does_not_overwrite_other_pie() {
        let db = SqliteDb::new(":memory:").await.unwrap();

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
    }

    #[tokio::test]
    async fn test_add_and_get_proof_for_multiple_blocks() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

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
    }

    #[tokio::test]
    async fn test_get_pie_returns_error_for_missing_block() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

        let result = db.get_pie(99, Step::Snos).await;
        assert!(
            result.is_err(),
            "Expected error when getting pie for non-existent block"
        );
    }

    #[tokio::test]
    async fn test_get_proof_returns_error_for_missing_block() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

        let result = db.get_proof(99, Step::Snos).await;
        assert!(
            result.is_err(),
            "Expected error when getting proof for non-existent block"
        );
    }

    #[tokio::test]
    async fn test_add_and_get_query_id_for_multiple_blocks() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

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
    }

    #[tokio::test]
    async fn test_query_id_does_not_overwrite_other_query_ids() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

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
    }

    #[tokio::test]
    async fn test_set_and_get_status() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

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
    }

    #[tokio::test]
    async fn test_get_status_returns_error_for_missing_block() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

        let result = db.get_status(99).await;
        assert!(
            result.is_err(),
            "Expected error when getting status for non-existent block"
        );
    }

    #[tokio::test]
    async fn test_remove_block_deletes_pies_and_proofs() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

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
    }

    #[tokio::test]
    async fn test_add_and_get_failed_block() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

        db.initialize_block(1).await.unwrap();
        db.add_failed_block(1, "failed".to_string()).await.unwrap();

        let failed_blocks = db.get_failed_blocks().await.unwrap();

        assert_eq!(failed_blocks, vec![(1, "failed".to_string())]);
    }

    #[tokio::test]
    async fn test_add_state_update_and_get_state_update() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

        db.initialize_block(1).await.unwrap();

        let state_update = starknet::core::types::StateUpdate {
            block_hash: Felt::ZERO,
            new_root: Felt::ZERO,
            old_root: Felt::ZERO,
            state_diff: StateDiff {
                storage_diffs: vec![],
                declared_classes: vec![],
                deployed_contracts: vec![],
                nonces: vec![],
                deprecated_declared_classes: vec![],
                replaced_classes: vec![],
            },
        };

        db.add_state_update(1, state_update.clone()).await.unwrap();
        let result = db.get_state_update(1).await.unwrap();

        assert_eq!(result.block_hash, state_update.block_hash);
        assert_eq!(result.new_root, state_update.new_root);
        assert_eq!(result.old_root, state_update.old_root);
    }

    #[tokio::test]
    async fn test_unique_constraint_enforced_on_pies() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

        db.initialize_block(1).await.unwrap();

        let pie1 = vec![1, 2, 3];
        let pie2 = vec![4, 5, 6];

        // Add first pie
        db.add_pie(1, pie1.clone(), Step::Snos).await.unwrap();
        let result1 = db.get_pie(1, Step::Snos).await.unwrap();
        assert_eq!(result1, pie1);

        // Update with second pie (should update, not create duplicate)
        db.add_pie(1, pie2.clone(), Step::Snos).await.unwrap();
        let result2 = db.get_pie(1, Step::Snos).await.unwrap();
        assert_eq!(result2, pie2);
    }

    #[tokio::test]
    async fn test_unique_constraint_enforced_on_proofs() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

        db.initialize_block(1).await.unwrap();

        let proof1 = vec![10, 20, 30];
        let proof2 = vec![40, 50, 60];

        // Add first proof
        db.add_proof(1, proof1.clone(), Step::Snos).await.unwrap();
        let result1 = db.get_proof(1, Step::Snos).await.unwrap();
        assert_eq!(result1, proof1);

        // Update with second proof (should update, not create duplicate)
        db.add_proof(1, proof2.clone(), Step::Snos).await.unwrap();
        let result2 = db.get_proof(1, Step::Snos).await.unwrap();
        assert_eq!(result2, proof2);
    }

    #[tokio::test]
    async fn test_unique_constraint_enforced_on_job_ids() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

        db.initialize_block(1).await.unwrap();

        let query_id_1 = "query_1".to_string();
        let query_id_2 = "query_2".to_string();

        // Add first query id
        db.add_query_id(1, query_id_1.clone(), Query::SnosProof)
            .await
            .unwrap();
        let result1 = db.get_query_id(1, Query::SnosProof).await.unwrap();
        assert_eq!(result1, query_id_1);

        // Update with second query id (should update, not create duplicate)
        db.add_query_id(1, query_id_2.clone(), Query::SnosProof)
            .await
            .unwrap();
        let result2 = db.get_query_id(1, Query::SnosProof).await.unwrap();
        assert_eq!(result2, query_id_2);
    }

    #[tokio::test]
    async fn test_unique_constraint_enforced_on_state_updates() {
        let db = SqliteDb::new(IN_MEMORY_DB).await.unwrap();

        db.initialize_block(1).await.unwrap();

        let state_update_1 = starknet::core::types::StateUpdate {
            block_hash: Felt::ZERO,
            new_root: Felt::ZERO,
            old_root: Felt::ZERO,
            state_diff: StateDiff {
                storage_diffs: vec![],
                declared_classes: vec![],
                deployed_contracts: vec![],
                nonces: vec![],
                deprecated_declared_classes: vec![],
                replaced_classes: vec![],
            },
        };

        let state_update_2 = starknet::core::types::StateUpdate {
            block_hash: Felt::ONE,
            new_root: Felt::ONE,
            old_root: Felt::ONE,
            state_diff: StateDiff {
                storage_diffs: vec![],
                declared_classes: vec![],
                deployed_contracts: vec![],
                nonces: vec![],
                deprecated_declared_classes: vec![],
                replaced_classes: vec![],
            },
        };

        // Add first state update
        db.add_state_update(1, state_update_1.clone())
            .await
            .unwrap();
        let result1 = db.get_state_update(1).await.unwrap();
        assert_eq!(result1.block_hash, state_update_1.block_hash);

        // Update with second state update (should update, not create duplicate)
        db.add_state_update(1, state_update_2.clone())
            .await
            .unwrap();
        let result2 = db.get_state_update(1).await.unwrap();
        assert_eq!(result2.new_root, state_update_2.new_root);
        assert_eq!(result2.old_root, state_update_2.old_root);
    }
}
