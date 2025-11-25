use assert_matches::assert_matches;
use katana_primitives::block::{
    Block, BlockHashOrNumber, FinalityStatus, Header, SealedBlockWithStatus,
};
use katana_primitives::transaction::TxType;
use katana_primitives::{address, felt, ContractAddress};
use katana_provider::api::block::{
    BlockHashProvider, BlockNumberProvider, BlockProvider, BlockWriter,
};
use katana_provider::api::state::StateFactoryProvider;
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::transaction::{ReceiptProvider, TransactionProvider};
use katana_provider::{ForkProviderFactory, MutableProvider, ProviderError, ProviderFactory};
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_types::MerkleNode;

const SEPOLIA_RPC_URL: &str = "https://api.cartridge.gg/x/starknet/sepolia";
const FORK_BLOCK_NUMBER: u64 = 2888618;

#[tokio::test]
async fn forked_provider_latest_number() {
    let fork_block_number = 2906771;
    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());

    let provider_factory = ForkProviderFactory::new_in_memory(fork_block_number, starknet_client);
    let provider_mut = provider_factory.provider_mut();

    let expected_latest_number = fork_block_number;
    let actual_latest_number = provider_mut.latest_number().unwrap();

    assert_eq!(actual_latest_number, expected_latest_number);

    let new_block_number = fork_block_number + 1;

    provider_mut
        .insert_block_with_states_and_receipts(
            SealedBlockWithStatus {
                block: Block {
                    header: Header { number: new_block_number, ..Default::default() },
                    body: Vec::new(),
                }
                .seal(),
                status: FinalityStatus::AcceptedOnL2,
            },
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .unwrap();

    provider_mut.commit().unwrap();
    let provider = provider_factory.provider();

    let expected_latest_number = new_block_number;
    let actual_latest_number = provider.latest_number().unwrap();

    assert_eq!(actual_latest_number, expected_latest_number);
}

/// Test that the ForkedProvider can fetch block data from the forked network.
///
/// This test validates that when we request a block that exists on the forked network
/// (before the fork point), the provider successfully fetches it from the remote network.
#[tokio::test]
async fn block_from_forked_network() {
    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());
    let provider_factory = ForkProviderFactory::new_in_memory(FORK_BLOCK_NUMBER, starknet_client);

    let provider = provider_factory.provider();

    // Request a block that should exist on the forked network (before the fork point)
    // Using a block number that is well before the fork point
    let block_num = 2888610;
    let block_id = BlockHashOrNumber::Num(block_num);

    let result = provider.block(block_id).unwrap();
    let block = result.expect("block should exist");

    let expected_parent_hash =
        felt!("0x4abdd03c515d0513bc41d923987fd8cf977d9849252bb21777d90542ce7d8");
    let expected_sequencer_address =
        felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8");
    let expected_state_root =
        felt!("0x2a366576ae1fc1a028bdc1a669c4c0da01e858671576fd1fe92340f64990300");
    let expected_timestamp = 1763171340;
    let expected_tx_count = 13;

    assert_eq!(block.header.number, block_num);
    assert_eq!(block.header.parent_hash, expected_parent_hash);
    assert_eq!(block.header.sequencer_address, expected_sequencer_address);
    assert_eq!(block.header.state_root, expected_state_root);
    assert_eq!(block.header.timestamp, expected_timestamp);
    assert_eq!(block.body.len(), expected_tx_count);

    // assert that all related data is populated and can be fetched correctly

    let transactions = provider.transactions_by_block(block_id).unwrap();
    let transactions = transactions.expect("block transactions must be stored");
    assert_eq!(transactions.len(), expected_tx_count);

    let receipts = provider.receipts_by_block(block_id).unwrap();
    let receipts = receipts.expect("block transactions must be stored");
    assert_eq!(receipts.len(), expected_tx_count);

    let state_updates = provider.state_update(block_id).unwrap();
    assert!(state_updates.is_some());
}

#[tokio::test]
async fn block_hash_from_forked_network() {
    let expected_hash = felt!("0x4f3db32fa485be6e8ed6ac7ce715a8739e9a28d67ea575c502e25036b5f178a");

    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());
    let provider_factory = ForkProviderFactory::new_in_memory(FORK_BLOCK_NUMBER, starknet_client);
    let provider = provider_factory.provider();

    let block_num = 2888611;
    let result = provider.block_hash_by_num(block_num).unwrap();
    let block_hash = result.expect("block hash should exist");

    assert_eq!(block_hash, expected_hash);
}

#[tokio::test]
async fn block_after_fork_point_returns_none() {
    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());

    let provider_factory = ForkProviderFactory::new_in_memory(FORK_BLOCK_NUMBER, starknet_client);
    let provider = provider_factory.provider();

    // Request a block after the fork point (should not exist locally)
    // The block might exist on the forked network, but since it's after the fork point,
    // the provider should not fetch it and should return None.
    let block_num = FORK_BLOCK_NUMBER + 10;
    let block_id = BlockHashOrNumber::Num(block_num);

    let block = provider.block(block_id).unwrap();
    assert!(block.is_none(), "Block after fork point should return None");
}

#[tokio::test]
async fn transaction_from_forked_network() {
    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());
    let provider_factory = ForkProviderFactory::new_in_memory(FORK_BLOCK_NUMBER, starknet_client);
    let provider = provider_factory.provider();

    let block_id = BlockHashOrNumber::Num(2888610);
    let tx_hash = felt!("0x40042d86e1b52896f3c695b713f3114ca53905890df0e14d09b4c1d51e2b1b0");

    let result = provider.transaction_by_hash(tx_hash).unwrap();
    let tx = result.expect("tx should exist");

    assert_eq!(tx.r#type(), TxType::Invoke);
    // TODO: assert individual fields of the transaction

    // the related block should be fetched too.
    // assert that all related data is populated and can be fetched correctly.

    let forked_db = provider.forked_db().db().provider(); // bypass the ForkedProvider

    let result = forked_db.block(block_id).unwrap();
    let block = result.expect("block should be populated");

    let expected_parent_hash =
        felt!("0x4abdd03c515d0513bc41d923987fd8cf977d9849252bb21777d90542ce7d8");
    let expected_sequencer_address =
        felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8");
    let expected_state_root =
        felt!("0x2a366576ae1fc1a028bdc1a669c4c0da01e858671576fd1fe92340f64990300");
    let expected_timestamp = 1763171340;
    let expected_tx_count = 13;

    assert_eq!(block.header.number, 2888610);
    assert_eq!(block.header.parent_hash, expected_parent_hash);
    assert_eq!(block.header.sequencer_address, expected_sequencer_address);
    assert_eq!(block.header.state_root, expected_state_root);
    assert_eq!(block.header.timestamp, expected_timestamp);
    assert_eq!(block.body.len(), expected_tx_count);

    let transactions = forked_db.transactions_by_block(block_id).unwrap();
    let transactions = transactions.expect("block transactions must be stored");
    assert_eq!(transactions.len(), expected_tx_count);

    let receipts = forked_db.receipts_by_block(block_id).unwrap();
    let receipts = receipts.expect("block transactions must be stored");
    assert_eq!(receipts.len(), expected_tx_count);

    let state_updates = forked_db.state_update(block_id).unwrap();
    assert!(state_updates.is_some());
}

#[tokio::test]
async fn latest_fork_state() {
    let fork_block_number = 2906771;

    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());
    let provider_factory = ForkProviderFactory::new_in_memory(fork_block_number, starknet_client);
    let provider = provider_factory.provider();

    // because we forked at block 2906771, `provider.latest()` will return state at block 2906771
    let state = provider.latest().unwrap();

    // Class declared at block 2892448
    // https://sepolia.voyager.online/class/0x00e022115a73679D4E215Da00F53D8f681F5C52B488bf18C71fEA115e92181b1
    let class_hash = felt!("0x00e022115a73679d4e215da00f53d8f681f5c52b488bf18c71fea115e92181b1");
    let result1 = state.class(class_hash).unwrap();
    let result2 = state.compiled_class_hash_of_class_hash(class_hash).unwrap();

    assert!(result1.is_some());
    assert!(result2.is_some());

    // Contract deployed at block 2906741
    // https://sepolia.voyager.online/contract/0x0164b86b8fC5C0c84d3c53Bc95760F290420Ea2a32ed49A44fd046683a1CaAc2#readStorage
    let address = address!("0x0164b86b8fC5C0c84d3c53Bc95760F290420Ea2a32ed49A44fd046683a1CaAc2");
    let result1 = state.nonce(address).unwrap().expect("must exist");
    let result2 = state.class_hash_of_contract(address).unwrap().expect("must exist");

    assert_eq!(result1, felt!("0x0"));
    assert_eq!(result2, felt!("0xe824b9f2aa225812cf230d276784b99f182ec95066d84be90cd1682e4ad069"));
}

#[tokio::test]
async fn historical_fork_state() {
    let fork_block_number = 2906771;

    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());
    let provider_factory = ForkProviderFactory::new_in_memory(fork_block_number, starknet_client);
    let provider = provider_factory.provider();

    ////////////////////////////////////////////////////////////////////////////////////
    // Class
    ////////////////////////////////////////////////////////////////////////////////////

    let declared_block = 2892448;
    let class_hash = felt!("0x00e022115a73679d4e215da00f53d8f681f5c52b488bf18c71fea115e92181b1");

    // class must not exist before the block it was declared in

    let block_id = BlockHashOrNumber::Num(declared_block - 1);
    let state = provider.historical(block_id).unwrap().expect("historical state must exist");

    let result1 = state.class(class_hash).unwrap();
    let result2 = state.compiled_class_hash_of_class_hash(class_hash).unwrap();

    assert!(result1.is_none());
    assert!(result2.is_none());

    // class must exist at the block it was declared in

    let block_id = BlockHashOrNumber::Num(declared_block);
    let state = provider.historical(block_id).unwrap().expect("historical state must exist");

    let result1 = state.class(class_hash).unwrap();
    let result2 = state.class(class_hash).unwrap();

    assert!(result1.is_some());
    assert!(result2.is_some());

    // class must exist after the block it was declared in

    let block_id = BlockHashOrNumber::Num(declared_block + 1);
    let state = provider.historical(block_id).unwrap().expect("historical state must exist");

    let result1 = state.class(class_hash).unwrap();
    let result2 = state.class(class_hash).unwrap();

    assert!(result1.is_some());
    assert!(result2.is_some());

    ////////////////////////////////////////////////////////////////////////////////////
    // Contract
    ////////////////////////////////////////////////////////////////////////////////////

    let contract_deployed_block = 2906741; // the block the contract was deployed in
    let address = address!("0x0164b86b8fC5C0c84d3c53Bc95760F290420Ea2a32ed49A44fd046683a1CaAc2");

    // contract must not exist before the block it was deployed in

    let block_id = BlockHashOrNumber::Num(contract_deployed_block - 1);
    let state = provider.historical(block_id).unwrap().expect("historical state must exist");

    let result1 = state.nonce(address).unwrap();
    let result2 = state.class_hash_of_contract(address).unwrap();

    assert!(result1.is_none());
    assert!(result2.is_none());

    // contract must exist at the block it was deployed in

    let block_id = BlockHashOrNumber::Num(contract_deployed_block);
    let state = provider.historical(block_id).unwrap().expect("historical state must exist");

    let result1 = state.nonce(address).unwrap();
    let result2 = state.class_hash_of_contract(address).unwrap();

    assert!(result1.is_some());
    assert!(result2.is_some());

    // contract must exist after the block it was deployed in

    let block_id = BlockHashOrNumber::Num(contract_deployed_block + 1);
    let state = provider.historical(block_id).unwrap().expect("historical state must exist");

    let result1 = state.nonce(address).unwrap();
    let result2 = state.class_hash_of_contract(address).unwrap();

    assert!(result1.is_some());
    assert!(result2.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn pre_fork_state_proof() {
    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());

    // always use the latest block number of the forked chain because most nodes may not support
    // proofs for too old blocks
    //
    // we take the previous block because there were some instances where the latest block was not
    // available or supported by the node.
    let latest_block_number = starknet_client.block_number().await.unwrap().block_number - 1;
    let provider_factory =
        ForkProviderFactory::new_in_memory(latest_block_number, starknet_client.clone());
    let provider = provider_factory.provider();

    let state = provider.latest().unwrap();

    let classes = vec![felt!("0x00e022115a73679d4e215da00f53d8f681f5c52b488bf18c71fea115e92181b1")];
    let proofs = state.class_multiproof(classes.clone()).unwrap();

    let expected_proofs = starknet_client
        .get_storage_proof(latest_block_number.into(), Some(classes), None, None)
        .await
        .unwrap();

    // TODO: assert the nodes ordering - ensure they are in the same order. currently, pathfinder
    // doesn't return the nodes in the same order as katana.
    assert_eq!(proofs.0.len(), expected_proofs.classes_proof.nodes.len());
    for expected_node in expected_proofs.classes_proof.nodes.0.into_iter() {
        let node_hash = expected_node.node_hash;
        let actual_node = proofs.0.get(&node_hash).cloned().map(MerkleNode::from);
        assert_eq!(Some(expected_node.node), actual_node)
    }

    let contracts =
        vec![address!("0x04f4e29add19afa12c868ba1f4439099f225403ff9a71fe667eebb50e13518d3")];
    let proofs = state.contract_multiproof(contracts.clone()).unwrap();

    let expected_proofs = starknet_client
        .get_storage_proof(latest_block_number.into(), None, Some(contracts), None)
        .await
        .unwrap();

    // TODO: assert the nodes ordering - ensure they are in the same order. currently, pathfinder
    // doesn't return the nodes in the same order as katana.
    assert_eq!(proofs.0.len(), expected_proofs.contracts_proof.nodes.len());
    for expected_node in expected_proofs.contracts_proof.nodes.0.into_iter() {
        let node_hash = expected_node.node_hash;
        let actual_node = proofs.0.get(&node_hash).cloned().map(MerkleNode::from);
        assert_eq!(Some(expected_node.node), actual_node)
    }
}

#[tokio::test]
async fn post_fork_state_proof_should_not_be_supported() {
    let fork_block_number = 2906771;

    let starknet_client = StarknetClient::new(SEPOLIA_RPC_URL.try_into().unwrap());
    let provider_factory = ForkProviderFactory::new_in_memory(fork_block_number, starknet_client);

    let provider_mut = provider_factory.provider_mut();

    let new_block_number = fork_block_number + 1;
    provider_mut
        .insert_block_with_states_and_receipts(
            SealedBlockWithStatus {
                block: Block {
                    header: Header { number: new_block_number, ..Default::default() },
                    body: Vec::new(),
                }
                .seal(),
                status: FinalityStatus::AcceptedOnL2,
            },
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .unwrap();
    provider_mut.commit().unwrap();

    let provider = provider_factory.provider();
    let state = provider.latest().unwrap();

    let class_hash = felt!("0x00e022115a73679d4e215da00f53d8f681f5c52b488bf18c71fea115e92181b1");
    let result = state.class_multiproof(vec![class_hash]);
    assert_matches!(result, Err(ProviderError::StateProofNotSupported));

    let address = address!("0x0164b86b8fC5C0c84d3c53Bc95760F290420Ea2a32ed49A44fd046683a1CaAc2");
    let result = state.contract_multiproof(vec![address]);
    assert_matches!(result, Err(ProviderError::StateProofNotSupported));
}
