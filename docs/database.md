# Database

## Table layout

```mermaid
erDiagram

Headers {
    KEY BlockNumber
    VALUE Header
}

BlockStateUpdates {
    KEY BlockNumber
    VALUE StateUpdates
}

BlockHashes {
    KEY BlockNumber
    VALUE BlockHash
}

BlockNumbers {
    KEY BlockHash
    VALUE BlockNumber
}

BlockStatusses {
    KEY BlockNumber
    VALUE FinalityStatus
}

BlockBodyIndices {
    KEY BlockNumber
    VALUE StoredBlockBodyIndices
}

TxNumbers {
    KEY TxHash
    VALUE TxNumber
}

TxHashes {
    KEY TxNumber
    VALUE TxHash
}

TxTraces {
    KEY TxNumber
    VALUE TxExecInfo
}

Transactions {
    KEY TxNumber
    VALUE Tx
}

TxBlocks {
    KEY TxNumber
    VALUE BlockNumber
}

Receipts {
    KEY TxNumber
    VALUE ReceiptEnvelope
}

CompiledClassHashes {
    KEY ClassHash
    VALUE CompiledClassHash
}

CompiledContractClasses {
    KEY ClassHash
    VALUE StoredContractClass
}

SierraClasses {
    KEY ClassHash
    VALUE FlattenedSierraClass
}

ContractInfo {
    KEY ContractAddress
    VALUE GenericContractInfo
}

ContractStorage {
    KEY ContractAddress
    DUP_KEY StorageKey
    VALUE StorageEntry
}

ClassDeclarationBlock {
    KEY ClassHash
    VALUE BlockNumber
}

ClassDeclarations {
    KEY BlockNumber
    DUP_KEY ClassHash
    VALUE ClassHash
}

ContractInfoChangeSet {
    KEY ContractAddress
    VALUE ContractInfoChangeList
}

NonceChangeHistory {
    KEY BlockNumber
    DUP_KEY ContractAddress
    VALUE ContractNonceChange
}

ClassChangeHistory {
    KEY BlockNumber
    DUP_KEY ContractAddress
    VALUE ContractClassChange
}

StorageChangeSet {
    KEY ContractStorageKey
    VALUE BlockList
}

StorageChangeHistory {
    KEY BlockNumber
    DUP_KEY ContractStorageKey
    VALUE ContractStorageEntry
}


BlockHashes ||--|| BlockNumbers : "block id"
BlockNumbers ||--|| BlockBodyIndices : "has"
BlockNumbers ||--|| Headers : "has"
BlockNumbers ||--|| BlockStateUpdates : "has canonical state diff"
BlockNumbers ||--|| BlockStatusses : "has"

BlockBodyIndices ||--o{ Transactions : "block txs"

TxHashes ||--|| TxNumbers : "tx id"
TxNumbers ||--|| Transactions : "has"
TxBlocks ||--|{ Transactions : "tx block"
Transactions ||--|| Receipts : "each tx must have a receipt"
Transactions ||--|| TxTraces : "each tx must have a trace"

CompiledClassHashes ||--|| CompiledContractClasses : "has"
CompiledClassHashes ||--|| SierraClasses : "has"
SierraClasses |o--|| CompiledContractClasses : "has"

ContractInfo ||--o{ ContractStorage : "a contract storage slots"
ContractInfo ||--|| CompiledClassHashes : "has"

ContractInfo }|--|{ ContractInfoChangeSet : "has"
ContractStorage }|--|{ StorageChangeSet : "has"
ContractInfoChangeSet }|--|{ NonceChangeHistory : "has"
ContractInfoChangeSet }|--|{ ClassChangeHistory : "has"
CompiledClassHashes ||--|| ClassDeclarationBlock : "has"
ClassDeclarationBlock ||--|| ClassDeclarations : "has"
BlockNumbers ||--|| ClassDeclarations : ""
StorageChangeSet }|--|{ StorageChangeHistory : "has"
```

New receipt rows are stored as a receipt-specific envelope plus `zstd(postcard(receipt))`.
Legacy rows without that envelope remain readable and are decoded as raw postcard bytes for
backward compatibility.

## Envelope Header Convention

When a table value needs format evolution without a full migration, use an explicit envelope
header:

`[magic:4][version:1][encoding:1][payload...]`

The `magic` field convention for Katana DB envelopes is:

- 4-byte uppercase ASCII.
- First byte is `K` (Katana DB namespace).
- Remaining 3 bytes identify the payload family.

For receipts we use `KRCP` (`K` + `RCP`).

Reader behavior for envelope-enabled values:

- If magic matches, treat the row as enveloped and validate `version` + `encoding`.
- If magic does not match, treat the row as legacy format.
- If magic matches but metadata is unsupported/corrupt, return an error (do not fall back to
  legacy decoding).

`BlockStateUpdates` stores the canonical per-block state diff used by `StateUpdateProvider` and RPC `get_state_update`.

The `*ChangeHistory`, `*ChangeSet`, `ClassDeclarations`, and `MigratedCompiledClassHashes` tables are historical reconstruction data. They may be compacted by pruning and must not be treated as the canonical source of a block's exact state diff.
