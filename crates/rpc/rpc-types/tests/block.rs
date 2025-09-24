use assert_matches::assert_matches;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::{address, felt, ContractAddress};
use katana_rpc_types::block::{BlockWithReceipts, BlockWithTxHashes, BlockWithTxs};
use serde_json::Value;
use starknet::core::types::ResourcePrice;

mod fixtures;

#[test]
fn preconfirmed_block_with_tx_hashes() {
    let json = fixtures::test_data::<Value>("v0.9/blocks/preconfirmed_with_tx_hashes.json");
    let block: BlockWithTxHashes = serde_json::from_value(json.clone()).unwrap();

    assert_matches!(&block, BlockWithTxHashes::PreConfirmed(block) => {
        assert_eq!(block.block_number, 1833173);
        assert_eq!(block.l1_da_mode, L1DataAvailabilityMode::Blob);

        assert_eq!(block.sequencer_address, address!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"));
        assert_eq!(block.timestamp, 1756770839);
        assert_eq!(block.starknet_version, "0.14.0");

        assert_eq!(block.l1_data_gas_price, ResourcePrice { price_in_fri: felt!("0x8403"), price_in_wei: felt!("0x1") });
        assert_eq!(block.l1_gas_price, ResourcePrice { price_in_fri: felt!("0x1ed0472629d4"), price_in_wei: felt!("0x3bc0fd69") });
        assert_eq!(block.l2_gas_price, ResourcePrice { price_in_fri: felt!("0xb2d05e00"), price_in_wei: felt!("0x15ac1") });

        assert_eq!(block.transactions, vec![
            felt!("0x47c73edc595bcd95f804d3a6d52fb5178de9465e64c9598908c24744cd40120"),
            felt!("0x5555ec91184b0c219569e040bbd939af698dbc69b5a017ba63550ee1441b40f"),
            felt!("0x3ac08f22812508465f97c583bba2ca6d75c05484e8fb4b43761fd85ae4bbf3c"),
        ]);
    });

    let serialized = serde_json::to_value(&block).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn confirmed_block_with_tx_hashes() {
    let json = fixtures::test_data::<Value>("v0.9/blocks/confirmed_with_tx_hashes.json");
    let block: BlockWithTxHashes = serde_json::from_value(json.clone()).unwrap();

    assert_matches!(&block, BlockWithTxHashes::Confirmed(block) => {
        assert_eq!(block.block_hash, felt!("0x6370ed4eb1232947c90ad4432baf9e4efa34ca721a3ba26e190e96aa098e27a"));
        assert_eq!(block.block_number, 1833241);
        assert_eq!(block.l1_da_mode, L1DataAvailabilityMode::Blob);
        assert_eq!(block.new_root, felt!("0x2375756642af920651d46364bf5c96c1bf6f6f73cdc2c28e5ecf8d1f56f04bc"));
        assert_eq!(block.parent_hash, felt!("0x2e669d6c7d0843aeadabb9a187a8e3381e07f71d9213ef7cbd25f78c40447b4"));

        assert_eq!(block.sequencer_address, address!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"));
        assert_eq!(block.timestamp, 1756771188);
        assert_eq!(block.starknet_version, "0.14.0");

        assert_eq!(block.l1_data_gas_price, ResourcePrice { price_in_fri: felt!("0x8403"), price_in_wei: felt!("0x1") });
        assert_eq!(block.l1_gas_price, ResourcePrice { price_in_fri: felt!("0x1ed58f4583b6"), price_in_wei: felt!("0x3bcb3b74") });
        assert_eq!(block.l2_gas_price, ResourcePrice { price_in_fri: felt!("0xb2d05e00"), price_in_wei: felt!("0x15ac1") });

        assert_eq!(block.transactions, vec![
            felt!("0x10a76b2ed3aadc7b66dfd9c0709439f1f993a6982afb1cd861e1a3f54ae9e84"),
            felt!("0x3161178b1643756f813719d98590075870bce753d703c4f6923e04cdba8a53b"),
            felt!("0x730939974ef7d9a3fea231dd2d5bcfe866c688251e7ca730fa7872f40ee86a4"),
            felt!("0x70004542093386020f23a4dc0507a5cc29f272a4c3944ddd3d78cd4dde588c0"),
            felt!("0x70ac8e8f14ceef2bbf2fcc1fe3edab0e8f76e2797f9c2407e335ade25aac15e"),
            felt!("0x68947c8ac52372a560bbef1ab0f1be46d395c2a1e595fd71a710c80f19804a2"),
            felt!("0x756015c0be51ac3fca9ff6f8364f2314614ddf4072da87b4ea2a8308a013428"),
            felt!("0xbfad9b6e7bcd4dcb814774065d2625f40b8d4f16ace7709b66c89bd5646f51"),
            felt!("0x6e80b4adebf3936985334ad4fdef292740c0d29a4df75eb1c168db6dcc1eb29"),
        ]);
    });

    let serialized = serde_json::to_value(&block).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn preconfirmed_block_with_txs() {
    let json = fixtures::test_data::<Value>("v0.9/blocks/preconfirmed_with_txs.json");
    let block: BlockWithTxs = serde_json::from_value(json.clone()).unwrap();

    assert_matches!(&block, BlockWithTxs::PreConfirmed(block) => {
        assert_eq!(block.block_number, 1833278);
        assert_eq!(block.l1_da_mode, L1DataAvailabilityMode::Blob);

        assert_eq!(block.sequencer_address, address!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"));
        assert_eq!(block.timestamp, 1756771378);
        assert_eq!(block.starknet_version, "0.14.0");

        assert_eq!(block.l1_data_gas_price, ResourcePrice { price_in_fri: felt!("0x847a"), price_in_wei: felt!("0x1") });
        assert_eq!(block.l1_gas_price, ResourcePrice { price_in_fri: felt!("0x1ef67e023a24"), price_in_wei: felt!("0x3bd53001") });
        assert_eq!(block.l2_gas_price, ResourcePrice { price_in_fri: felt!("0xb2d05e00"), price_in_wei: felt!("0x1598a") });

        assert_eq!(block.transactions.len(), 8);
    });

    let serialized = serde_json::to_value(&block).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn confirmed_block_with_txs() {
    let json = fixtures::test_data::<Value>("v0.9/blocks/confirmed_with_txs.json");
    let block: BlockWithTxs = serde_json::from_value(json.clone()).unwrap();

    assert_matches!(&block, BlockWithTxs::Confirmed(block) => {
        assert_eq!(block.block_hash, felt!("0x28357304a645764b85790bf6d138be6fa25a53e2a11a5014edca4bdc24d0a2f"));
        assert_eq!(block.block_number, 1833278);
        assert_eq!(block.l1_da_mode, L1DataAvailabilityMode::Blob);
        assert_eq!(block.new_root, felt!("0x50035a2a56b6ba00f4077de6601d1212fc7aff8b98b29dd4d0966d8fb9b822c"));
        assert_eq!(block.parent_hash, felt!("0x19b9f657c1c574cc007b190ab98ebb8499af14b8fccaf5aee56cc60e3a14f7b"));

        assert_eq!(block.sequencer_address, address!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"));
        assert_eq!(block.timestamp, 1756771378);
        assert_eq!(block.starknet_version, "0.14.0");

        assert_eq!(block.l1_data_gas_price, ResourcePrice { price_in_fri: felt!("0x847a"), price_in_wei: felt!("0x1") });
        assert_eq!(block.l1_gas_price, ResourcePrice { price_in_fri: felt!("0x1ef67e023a24"), price_in_wei: felt!("0x3bd53001") });
        assert_eq!(block.l2_gas_price, ResourcePrice { price_in_fri: felt!("0xb2d05e00"), price_in_wei: felt!("0x1598a") });

        assert_eq!(block.transactions.len(), 8);
    });

    let serialized = serde_json::to_value(&block).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn preconfirmed_block_with_receipts() {
    let json = fixtures::test_data::<Value>("v0.9/blocks/preconfirmed_with_receipts.json");
    let block: BlockWithReceipts = serde_json::from_value(json.clone()).unwrap();

    assert_matches!(&block, BlockWithReceipts::PreConfirmed(block) => {
        assert_eq!(block.block_number, 1833338);
        assert_eq!(block.l1_da_mode, L1DataAvailabilityMode::Blob);

        assert_eq!(block.sequencer_address, address!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"));
        assert_eq!(block.timestamp, 1756771686);
        assert_eq!(block.starknet_version, "0.14.0");

        assert_eq!(block.l1_data_gas_price, ResourcePrice { price_in_fri: felt!("0x847a"), price_in_wei: felt!("0x1") });
        assert_eq!(block.l1_gas_price, ResourcePrice { price_in_fri: felt!("0x1f060f310854"), price_in_wei: felt!("0x3bf344fd") });
        assert_eq!(block.l2_gas_price, ResourcePrice { price_in_fri: felt!("0xb2d05e00"), price_in_wei: felt!("0x1598a") });

        assert_eq!(block.transactions.len(), 6);
    });

    let serialized = serde_json::to_value(&block).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn confirmed_block_with_receipts() {
    let json = fixtures::test_data::<Value>("v0.9/blocks/confirmed_with_receipts.json");
    let block: BlockWithReceipts = serde_json::from_value(json.clone()).unwrap();

    assert_matches!(&block, BlockWithReceipts::Confirmed(block) => {
        assert_eq!(block.block_hash, felt!("0x3ced593414cc5dbf9f0a29c55d208a14b60742a16937cf106842f7ab77ff7cb"));
        assert_eq!(block.block_number, 1833335);
        assert_eq!(block.l1_da_mode, L1DataAvailabilityMode::Blob);
        assert_eq!(block.new_root, felt!("0x27b1ca40480add785debbc4e445af2f90da1656edaff4d652429a4b58261762"));
        assert_eq!(block.parent_hash, felt!("0x1d64e38fa4a55d37ad28f428c2c755820cf6049537e6df9bdb0c30f3bd0156"));

        assert_eq!(block.sequencer_address, address!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"));
        assert_eq!(block.timestamp, 1756771671);
        assert_eq!(block.starknet_version, "0.14.0");

        assert_eq!(block.l1_data_gas_price, ResourcePrice { price_in_fri: felt!("0x847a"), price_in_wei: felt!("0x1") });
        assert_eq!(block.l1_gas_price, ResourcePrice { price_in_fri: felt!("0x1f0532469d43"), price_in_wei: felt!("0x3bf19a17") });
        assert_eq!(block.l2_gas_price, ResourcePrice { price_in_fri: felt!("0xb2d05e00"), price_in_wei: felt!("0x1598a") });

        assert_eq!(block.transactions.len(), 7);
    });

    let serialized = serde_json::to_value(&block).unwrap();
    assert_eq!(serialized, json);
}
