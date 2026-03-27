# Syncing pipeline

## Stages

The pipeline is composed of the following stages, executed in order for each chunk of blocks:

| Stage | ID | Description |
|-------|----|-------------|
| **Blocks** | `Blocks` | Downloads blocks from the sync source (JSON-RPC or gateway), validates chain invariants and block hashes, and stores block data (headers, hashes, body indices, canonical state updates, transactions, receipts, traces, class artifacts, declarations). Does **not** build historical state indices. |
| **Classes** | `Classes` | Downloads full class artifacts (Sierra / legacy) for any classes declared in the synced blocks that are not yet stored locally. |
| **IndexHistory** | `IndexHistory` | Reads the canonical `BlockStateUpdates` written by the Blocks stage and builds historical state indices: `ContractStorage`, `StorageChangeSet`, `StorageChangeHistory`, `ContractInfo`, `ContractInfoChangeSet`, `ClassChangeHistory`, `NonceChangeHistory`. Owns pruning of these indices. |
| **StateTrie** | `StateTrie` | Computes and validates state tries (contract, class, storage) for each block, verifying the computed state root matches the block header. Only runs when trie computation is enabled. |

> **Note:** The sequencing / block-production path and `ForkedProvider` use `insert_block_with_states_and_receipts`, which calls both `insert_block_data` and `insert_state_history` in a single transaction. The pipeline separates these into distinct stages so that each concern can be checkpointed and pruned independently.

## Pipeline flow

```mermaid
flowchart TD
    A[Start Pipeline Run] --> B[Initialize chunk_tip]

    B --> D{Process Blocks in Chunks}
    D --> E[run_once_until]

    %% run_once_until subflow
    E --> S1[For each Stage]
    S1 --> S2[Get Stage Checkpoint]
    S2 --> S3{Checkpoint >= Target?}
    S3 -->|Yes| S4[Skip Stage]
    S3 -->|No| S5[Execute Stage<br>from checkpoint+1 to target]
    S5 --> S6[Update Stage Checkpoint]
    S6 --> S1
    S4 --> S1

    S1 -->|All Stages Complete| F{Reached Target Tip?}
    F -->|No| G[Increment chunk_tip by<br>chunk_size]
    G --> D

    F -->|Yes| H[Wait for New Tip]
    H -->|New Tip Received| D
    H -->|Channel Closed| I[Pipeline Complete]

    style A fill:#f9f,stroke:#333
    style I fill:#f96,stroke:#333

%% Example annotations
    classDef note fill:#fff,stroke:#333,stroke-dasharray: 5 5
    N1[For example: Tip=1000<br>chunk_size=100<br>Processes: 0-100, 100-200, etc]:::note
    N1 -.-> D
```
