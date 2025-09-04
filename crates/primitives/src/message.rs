use alloy_primitives::{Keccak256, B256, U256};

use crate::contract::{ContractAddress, Nonce};
use crate::eth::Address as EthAddress;
use crate::Felt;

/// Message from L1.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct L1ToL2Message {
    /// The address of the L1 contract sending the message
    pub from_address: EthAddress,
    /// The target L2 address the message is sent to
    pub to_address: ContractAddress,
    /// The selector of the l1_handler in invoke in the target contract
    pub entry_point_selector: Felt,
    /// The payload of the message
    pub payload: Vec<Felt>,
    /// The nonce of the message
    pub nonce: Nonce,
}

impl L1ToL2Message {
    /// Calculates the message hash.
    ///
    /// The hashing algorithm is based on the canonical Solidity [implementation] of Starknet core
    /// contract on Ethereum. Check out the Starknet [docs] on messaging for more details.
    ///
    /// [implementation]: https://github.com/starkware-libs/cairo-lang/blob/8276ac35830148a397e1143389f23253c8b80e93/src/starkware/starknet/solidity/StarknetMessaging.sol#L85-L106
    /// [docs]: https://docs.starknet.io/architecture/messaging/#l1_l2_message_hashing
    pub fn hash(&self) -> B256 {
        // Below is the Solidity implementation of the message hash calculation.
        //
        // function l1ToL2MsgHash(
        //     address fromAddress,
        //     uint256 toAddress,
        //     uint256 selector,
        //     uint256[] calldata payload,
        //     uint256 nonce
        // ) public pure returns (bytes32) {
        //     return
        //         keccak256(
        //             abi.encodePacked(
        //                 uint256(uint160(fromAddress)),
        //                 toAddress,
        //                 nonce,
        //                 selector,
        //                 payload.length,
        //                 payload
        //             )
        //         );
        // }

        let mut hasher = Keccak256::new();

        // fromAddress: Cast Ethereum address (20 bytes) to uint256 (32 bytes)
        // The address is padded with leading zeros to fill 32 bytes
        let from_address = U256::from_be_slice(self.from_address.0.as_slice());
        hasher.update(from_address.to_be_bytes::<{ U256::BYTES }>());

        // toAddress: Felt (32 bytes), no padding needed
        hasher.update(self.to_address.to_bytes_be());

        // nonce: Felt (32 bytes), no padding needed
        hasher.update(self.nonce.to_bytes_be());

        // selector: Felt (32 bytes), no padding needed
        hasher.update(self.entry_point_selector.to_bytes_be());

        // payload.length: Encode as uint256 (32 bytes)
        let payload_len = U256::from(self.payload.len());
        hasher.update(payload_len.to_be_bytes::<{ U256::BYTES }>());

        // payload: Each element is already a  (32 bytes)
        // Elements are concatenated sequentially with no additional padding
        for elem in &self.payload {
            hasher.update(elem.to_bytes_be());
        }

        hasher.finalize()
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::hex;

    use super::*;
    use crate::{address, felt};

    // L1Handler transaction on mainnet: https://voyager.online/tx/0x5e83f152aabc4caad08589339d2b4cfbc1b688d936ca3e7e919c50a7bab3ba4
    #[test]
    fn l1_to_l2_message_hash() {
        let expected_msg_hash =
            hex!("0x6708556b516f77ddecce43d49b7a125d73386ba411a3817fcf41d1bf7e16d3b0");

        let nonce = felt!("0x199007");

        let from_address =
            EthAddress::from_slice(&hex!("f6080d9fbeebcd44d89affbfd42f098cbff92816"));

        let to_address =
            address!("0x5cd48fccbfd8aa2773fe22c217e808319ffcc1c5a6a463f7d8fa2da48218196");

        let entry_point_selector =
            felt!("0x1b64b1b3b690b43b9b514fb81377518f4039cd3e4f4914d8a6bdf01d679fb19");

        let payload = vec![
            felt!("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"),
            felt!("0x25ef9386a96bde9bb47be826155efae7722148c0"),
            felt!("0x1dffda3b65102d41e645461ffb1abbff20769bad777631a26b7fbeca1b7c2fd"),
            felt!("0x2e90edd000"),
            felt!("0x0"),
        ];

        let msg = L1ToL2Message { from_address, to_address, entry_point_selector, payload, nonce };
        let actual_hash = msg.hash();

        assert_eq!(actual_hash.0, expected_msg_hash);
    }
}
