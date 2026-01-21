


## Rust clients 

We build two crates : 

(1) AMD TEE Registry Client : uses amd-sev-attestation-sdk (which includes SP1 program), and communicates with the amd_tee_registry on Starknet.

(2) Katana TEE Client : also uses amd sev sdk, but communicates with the Katana TEE contract. 

### The Katana TEE Client 

objectives : 

1. Communicates with a Katana TEE RPC endpoint.
    - Modification of Katana RPC to return also the device's certification (since it might not be available in KDS)

2. Send the attestation and the device certification to AMD TEE Registry Client create that will create the SP1 Proof, serialized as an array of Felt, for the amd_tee_registry contract. 
    - Map the RPC response from Katana to the amd-sev-snp-attestation SDK 's AttestationReport's struct. (in crates/amd-sev-snp-attestation-sdk/crates/sev-snp/src/report.rs)
    - Send this struct 
3. Serializes the SP1 proof for the @katana_tee contract along with necessary extra data. This contains the SP1 Proof and the 


### AMD TEE Registry Client :

objectives :

Interfaces with AMD public APIs to retrieve the necessary certificates
Interacts with the primary Starknet Smart Contract for the caching mechanism
Prepares attestation data for zero-knowledge proof generation using SP1 and generates the proof.
SP1 Program : crates/amd-sev-snp-attestation-sdk/crates/sp1-methods/sp1-verifier


Use conversion from TeeReport 

# GENERAL INSTRUCTIONS / Notes : 

Everything should be setup with a .env : Katana RPC URL, Starknet RPC URL, SP1 Network Key (no keys default : local stark to groth16 Proving if key is not present). 
Extract the minimum amount of crates from the amd-sev-snp-attestaion-sdk.
SP1 Version (might) need to be updated by garaga developers, assume the version is 5.2.1, as in Automata's SDK. It doesn't change anything except the Starknet Class hash used that will be updated by garaga devs.


# Context / Documentation / Resources : 
We are building a simplified subset (Only SP1) of AMD SEV Automata's SDK which are built for solidity. 
Garaga's groth16 proof to calldata generation : https://github.com/keep-starknet-strange/garaga/blob/main/tools/garaga_rs/src/calldata/full_proof_with_hints/groth16.rs


SP1 proof generation (done with )
SP1 Starknet DApp example (similar to the AMD_TEE_REgistry contract), but useful to look for the rust tooling example : https://github.com/feltroidprime/sp1-starknet-template 


RUST Starknet SDK : crates/starknet-rust

Starknet Environment variables are available in .env file, available variables names are presented in @.env.example. 

```
SEPOLIA_RPC_URL=
SEPOLIA_ACCOUNT_PRIVATE_KEY=
SEPOLIA_ACCOUNT_ADDRESS=

MAINNET_RPC_URL=
MAINNET_ACCOUNT_PRIVATE_KEY=
MAINNET_ACCOUNT_ADDRESS=
```
# DELIVERABLES 


### 2.2 Rust Crate

* A self-contained Rust crate that:
  * Communicates with a Katana TEE RPC endpoint.
  * Interfaces with AMD public APIs to retrieve the necessary certificates
  * Interacts with the primary Starknet Smart Contract for the caching mechanism
  * Prepares attestation data for zero-knowledge proof generation using SP1 and generates the proof
  * Serializes the SP1 proof for the secondary Starknet contract along with necessary extra data. 