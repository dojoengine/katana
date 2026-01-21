## 2. Deliverables

### 2.1 Starknet Smart Contract Components

* A primary Starknet smart contract implementing:
  * Proof verification for AMD SEV-SNP processor families, routed to Garaga’s SP1 Groth16 verifier.
  * Attestation and certificate verification logic, including on-chain caching where applicable.
* A Cairo library exposing verifications utilities regarding the attestation report. 
* A secondary, minimal Starknet Smart Contract representing a Katana downstream application that calls the primary contract and exposes the sequencer state. 

### 2.2 Rust Crate

* A self-contained Rust crate that:
  * Communicates with a Katana TEE RPC endpoint.
  * Interfaces with AMD public APIs to retrieve the necessary certificates
  * Interacts with the primary Starknet Smart Contract for the caching mechanism
  * Prepares attestation data for zero-knowledge proof generation using SP1 and generates the proof
  * Serializes the SP1 proof for the secondary Starknet contract along with necessary extra data. 

@clients/katana_tee_client :
@clients/amd_tee_registry_client :

### 2.3 End-to-End Reproducible Demonstration

* A complete end-to-end example deployed on Starknet Testnet, demonstrating:
  * Retrieval of TEE attestation data.
  * Zero-knowledge proof generation.
  * On-chain verification of the Katana sequencer state.

### 2.4 Upstream Improvements

* Where necessary, targeted improvements, modifications, or extensions to:
  * Katana’s repository,
  * Garaga’s SP1 verifier,
  * Automata Network’s AMD SEV-SNP zero-knowledge program

