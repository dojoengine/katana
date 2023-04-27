use std::{collections::HashMap, vec};

use blockifier::transaction::objects::TransactionExecutionInfo;
use starknet::providers::jsonrpc::models::TransactionStatus;
use starknet_api::{
    block::{BlockHash, BlockNumber},
    hash::StarkFelt,
    stark_felt,
    transaction::{
        DeclareTransactionOutput, DeployAccountTransactionOutput, DeployTransactionOutput, Event,
        InvokeTransactionOutput, L1HandlerTransactionOutput, MessageToL1, Transaction,
        TransactionHash, TransactionOutput, TransactionReceipt,
    },
};

#[derive(Debug)]
pub struct StarknetTransaction {
    pub inner: Transaction,
    pub status: TransactionStatus,
    pub block_hash: Option<BlockHash>,
    pub block_number: Option<BlockNumber>,
    pub execution_info: TransactionExecutionInfo,
}

impl StarknetTransaction {
    pub fn get_receipt(&self) -> TransactionReceipt {
        TransactionReceipt {
            output: self.get_output(),
            transaction_hash: self.inner.transaction_hash(),
            // pending / reverted txs shouldn't have a block number and hash
            block_number: self.block_number.unwrap_or(BlockNumber(0)),
            block_hash: self.block_hash.unwrap_or(BlockHash(stark_felt!(0))),
        }
    }

    pub fn get_output(&self) -> TransactionOutput {
        let events = self.get_emitted_events();
        match self.inner {
            Transaction::Invoke(_) => TransactionOutput::Invoke(InvokeTransactionOutput {
                events,
                actual_fee: self.execution_info.actual_fee,
                messages_sent: self.get_l2_to_l1_messages(),
            }),
            Transaction::Declare(_) => TransactionOutput::Declare(DeclareTransactionOutput {
                events,
                actual_fee: self.execution_info.actual_fee,
                messages_sent: self.get_l2_to_l1_messages(),
            }),
            Transaction::DeployAccount(_) => {
                TransactionOutput::DeployAccount(DeployAccountTransactionOutput {
                    events,
                    actual_fee: self.execution_info.actual_fee,
                    messages_sent: self.get_l2_to_l1_messages(),
                })
            }
            Transaction::L1Handler(_) => TransactionOutput::L1Handler(L1HandlerTransactionOutput {
                events,
                actual_fee: self.execution_info.actual_fee,
                messages_sent: self.get_l2_to_l1_messages(),
            }),
            Transaction::Deploy(_) => TransactionOutput::Deploy(DeployTransactionOutput {
                events,
                actual_fee: self.execution_info.actual_fee,
                messages_sent: self.get_l2_to_l1_messages(),
            }),
        }
    }

    pub fn get_emitted_events(&self) -> Vec<Event> {
        let mut events: Vec<Event> = vec![];

        if let Some(info) = self.execution_info.validate_call_info {
            events.extend(info.execution.events.iter().map(|e| Event {
                from_address: info.call.caller_address,
                content: e.event,
            }))
        }

        if let Some(info) = self.execution_info.execute_call_info {
            events.extend(info.execution.events.iter().map(|e| Event {
                from_address: info.call.caller_address,
                content: e.event,
            }))
        }

        if let Some(info) = self.execution_info.fee_transfer_call_info {
            events.extend(info.execution.events.iter().map(|e| Event {
                from_address: info.call.caller_address,
                content: e.event,
            }))
        }

        events
    }

    pub fn get_l2_to_l1_messages(&self) -> Vec<MessageToL1> {
        let mut messages: Vec<MessageToL1> = vec![];

        if let Some(info) = self.execution_info.validate_call_info {
            messages.extend(
                info.execution
                    .l2_to_l1_messages
                    .iter()
                    .map(|m| MessageToL1 {
                        payload: m.message.payload,
                        to_address: m.message.to_address,
                        from_address: info.call.caller_address,
                    }),
            )
        }

        if let Some(info) = self.execution_info.execute_call_info {
            messages.extend(
                info.execution
                    .l2_to_l1_messages
                    .iter()
                    .map(|m| MessageToL1 {
                        payload: m.message.payload,
                        to_address: m.message.to_address,
                        from_address: info.call.caller_address,
                    }),
            )
        }

        if let Some(info) = self.execution_info.fee_transfer_call_info {
            messages.extend(
                info.execution
                    .l2_to_l1_messages
                    .iter()
                    .map(|m| MessageToL1 {
                        payload: m.message.payload,
                        to_address: m.message.to_address,
                        from_address: info.call.caller_address,
                    }),
            )
        }

        messages
    }
}

pub struct StarknetTransactions {
    pub transactions: HashMap<TransactionHash, StarknetTransaction>,
}
