use std::{collections::HashMap, vec};

use anyhow::anyhow;
use blockifier::transaction::{
    errors::TransactionExecutionError, objects::TransactionExecutionInfo,
};
use starknet::core::types::TransactionStatus;
use starknet_api::{
    block::{BlockHash, BlockNumber},
    hash::StarkFelt,
    stark_felt,
    transaction::{
        DeclareTransactionOutput, DeployAccountTransactionOutput, DeployTransactionOutput, Event,
        Fee, InvokeTransactionOutput, L1HandlerTransactionOutput, MessageToL1, Transaction,
        TransactionHash, TransactionOutput, TransactionReceipt,
    },
};

#[derive(Debug)]
pub struct StarknetTransaction {
    pub inner: Transaction,
    pub status: TransactionStatus,
    pub block_hash: Option<BlockHash>,
    pub block_number: Option<BlockNumber>,
    pub execution_info: Option<TransactionExecutionInfo>,
    pub execution_error: Option<TransactionExecutionError>,
}

impl StarknetTransaction {
    pub fn new(
        inner: Transaction,
        status: TransactionStatus,
        execution_info: Option<TransactionExecutionInfo>,
        execution_error: Option<TransactionExecutionError>,
    ) -> Self {
        if status == TransactionStatus::Rejected && execution_error.is_none() {
            anyhow!("rejected transaction must have an execution error");
        };

        Self {
            inner,
            status,
            execution_info,
            execution_error,
            block_hash: None,
            block_number: None,
        }
    }

    pub fn actual_fee(&self) -> Fee {
        self.execution_info.map_or(Fee(0), |info| info.actual_fee)
    }

    pub fn get_receipt(&self) -> TransactionReceipt {
        TransactionReceipt {
            output: self.get_output(),
            transaction_hash: self.inner.transaction_hash(),
            block_number: self.block_number.unwrap_or(BlockNumber(0)),
            block_hash: self.block_hash.unwrap_or(BlockHash(stark_felt!(0))),
        }
    }

    pub fn get_emitted_events(&self) -> Vec<Event> {
        let mut events: Vec<Event> = vec![];

        let Some(execution_info) = self.execution_info else {
            return events;
        };

        if let Some(info) = execution_info.validate_call_info {
            events.extend(info.execution.events.iter().map(|e| Event {
                from_address: info.call.caller_address,
                content: e.event,
            }))
        }

        if let Some(info) = execution_info.execute_call_info {
            events.extend(info.execution.events.iter().map(|e| Event {
                from_address: info.call.caller_address,
                content: e.event,
            }))
        }

        if let Some(info) = execution_info.fee_transfer_call_info {
            events.extend(info.execution.events.iter().map(|e| Event {
                from_address: info.call.caller_address,
                content: e.event,
            }))
        }

        events
    }

    pub fn get_l2_to_l1_messages(&self) -> Vec<MessageToL1> {
        let mut messages: Vec<MessageToL1> = vec![];

        let Some(execution_info) = self.execution_info else {
            return messages;
        };

        if let Some(info) = execution_info.validate_call_info {
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

        if let Some(info) = execution_info.execute_call_info {
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

        if let Some(info) = execution_info.fee_transfer_call_info {
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

    fn get_output(&self) -> TransactionOutput {
        let actual_fee = self.actual_fee();
        let events = self.get_emitted_events();
        let messages_sent = self.get_l2_to_l1_messages();

        match self.inner {
            Transaction::Invoke(_) => TransactionOutput::Invoke(InvokeTransactionOutput {
                events,
                actual_fee,
                messages_sent,
            }),
            Transaction::Declare(_) => TransactionOutput::Declare(DeclareTransactionOutput {
                events,
                actual_fee,
                messages_sent,
            }),
            Transaction::DeployAccount(_) => {
                TransactionOutput::DeployAccount(DeployAccountTransactionOutput {
                    events,
                    actual_fee,
                    messages_sent,
                })
            }
            Transaction::L1Handler(_) => TransactionOutput::L1Handler(L1HandlerTransactionOutput {
                events,
                actual_fee,
                messages_sent,
            }),
            Transaction::Deploy(_) => TransactionOutput::Deploy(DeployTransactionOutput {
                events,
                actual_fee,
                messages_sent,
            }),
        }
    }
}

pub struct StarknetTransactions {
    pub transactions: HashMap<TransactionHash, StarknetTransaction>,
}
