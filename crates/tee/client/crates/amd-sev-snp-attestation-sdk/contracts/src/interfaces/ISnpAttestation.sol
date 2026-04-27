//SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.0;

enum ProcessorType {
    // 7003 series AMD EPYC Processor
    Milan,
    // 9004 series AMD EPYC Processor
    Genoa,
    // 97x4 series AMD EPYC Processor
    Bergamo,
    // 8004 series AMD EPYC Processor
    Siena
}

struct VerifierInput {
    uint64 timestamp;
    uint8 trustedCertsPrefixLen;
    bytes rawReport;
    bytes[] vekDerChain;
    // Global state verification (matches attestation report_data)
    bytes32 globalStateRoot;
    bytes32 contractsTreeRoot;
    bytes32 classesTreeRoot;
    // Contracts tree proof: verifies contract is in contracts_tree with expected storage_root
    bytes[] contractsProofNodes;
    bytes32 contractStorageRoot;  // Expected storage_root from contract leaf
    bytes32 contractClassHash;    // For computing contract leaf hash
    uint64 contractLeafNonce;     // Contract's nonce (different from replay protection nonce)
    // Storage proof: verifies keys/values are in contract's storage trie
    bytes[] storageKeys;
    bytes[] storageValues;
    bytes[] storageProofNodes;
    // Nonce-based replay protection (Ethereum-style): commitment = hash(storage_commitment, contractAddress, nonce, globalStateRoot)
    bytes32 contractAddress;
    uint64 nonce;
    // Fork block number: the L1/L2 block that Katana forked from.
    // TEE includes this in report_data: Poseidon(state_root, block_hash, fork_block_number, events_commitment).
    // 0 means non-fork mode.
    uint64 forkBlockNumber;
    // Event inclusion proof (C2: shard ending verification).
    // Proves a ShardFinished event exists in the block's events_commitment (Merkle root).
    // events_commitment is bound to TEE attestation via report_data.
    bytes32 eventsCommitment;
    bytes32 eventHash;
    uint32 eventIndex;
    uint32 eventsCount;
    bytes[] eventMerkleProof;    // Scale-encoded MultiProof (Poseidon hash, 64-bit keys)
    uint64 endBlockNumber;       // Block number where the event was found
}

struct VerifierJournal {
    VerificationResult result;
    uint64 timestamp;
    uint8 processorModel;
    bytes rawReport;
    bytes32[] certs;
    uint160[] certSerials;
    uint8 trustedCertsPrefixLen;
    // Commitment to verified storage (keccak256(abi.encode(keys, values))). 0 when no storage proof.
    bytes32 storageCommitment;
    // Fork block number from input, forwarded for on-chain verification.
    // 0 means non-fork mode.
    uint64 forkBlockNumber;
    // Block where ShardFinished event was proven by SP1 (0 = no event proof).
    uint64 endBlockNumber;
}

/// @dev Used to compute storage commitment: commitment = keccak256(abi.encode(StorageCommitmentInput(keys, values)))
struct StorageCommitmentInput {
    bytes[] keys;
    bytes[] values;
}

enum ZkCoProcessorType {
    None,
    RiscZero,
    Succinct,
    Pico
}

/**
 * @dev Enumeration of possible attestation verification results
 * Indicates the outcome of the verification process
 */
enum VerificationResult {
    // Attestation successfully verified
    Success,
    // Root certificate is not in the trusted set
    RootCertNotTrusted,
    // One or more intermediate certificates are not trusted
    IntermediateCertsNotTrusted,
    // Attestation timestamp is outside acceptable range
    InvalidTimestamp
}

/**
 * @title ZK Co-Processor Configuration Object
 * @param latestProgramIdentifier - This is the most up-to-date identifier of the ZK Program, required for
 * verification
 * @param defaultZkVerifier - Points to the address of a default ZK Verifier contract. Ideally
 * this should be pointing to a universal verifier, that may support multiple proof types and/or versions.
 */
struct ZkCoProcessorConfig {
    bytes32 latestProgramIdentifier;
    address defaultZkVerifier;
}

interface ISnpAttestation {
    // 51abd95c
    error Unknown_Zk_Coprocessor();
    // 105efc49
    error ZK_Route_Frozen(ZkCoProcessorType zkCoProcessor, bytes4 selector);
    // e147b0b2
    error Cannot_Remove_ProgramIdentifier(ZkCoProcessorType zkCoProcessor, bytes32 identifier);
    // 85ee11b0
    error Invalid_Program_Identifier(ZkCoProcessorType zkCoProcessor, bytes32 identifier);

    event AttestationSubmitted(VerificationResult result, ZkCoProcessorType zkCoProcessor, bytes output);

    /**
     * @param zkCoProcessorType 1 - RiscZero, 2 - Succinct, 3 - Pico... etc.
     * @return this is either the IMAGE_ID for RiscZero Guest Program or
     * Succiinct Program Verifying Key
     */
    function programIdentifier(ZkCoProcessorType zkCoProcessorType) external view returns (bytes32);

    /**
     * @notice get the default contract verifier for the provided ZK Co-processor
     */
    function zkVerifier(ZkCoProcessorType zkCoProcessorType) external view returns (address);

    /**
     * @notice gets the specific ZK Verifier for the provided ZK Co-processor and proof selector
     * @notice this function will revert if the provided selector has been frozen
     * @notice otherwise, if a specific ZK verifier is not configured for the provided selector
     * @notice it will return the default ZK verifier
     */
    function zkVerifier(ZkCoProcessorType zkCoProcessorType, bytes4 selector) external view returns (address);

    /**
     * @param zkCoProcessorType 1 - RiscZero, 2 - Succinct, 3 - Pico... etc.
     * @return this returns the list of all program identifiers for the specified ZK Co-processor
     */
    function programIdentifiers(ZkCoProcessorType zkCoProcessorType) external view returns (bytes32[] memory);

    /**
     * @notice Updates the Program Identifier for the specified ZK Co-Processor
     */
    function updateProgramIdentifier(ZkCoProcessorType zkCoProcessor, bytes32 identifier) external;

    /**
     * @notice Deprecates a Program Identifier for the specified ZK Co-Processor
     */
    function removeProgramIdentifier(ZkCoProcessorType zkCoProcessor, bytes32 identifier) external;

    /**
     * @notice Adds a verifier for a specific ZK Route to override the default ZK Verifier
     */
    function addVerifyRoute(ZkCoProcessorType zkCoProcessor, bytes4 selector, address verifier) external;

    /**
     * @notice PERMANENTLY freezes a ZK Route
     */
    function freezeVerifyRoute(ZkCoProcessorType zkCoProcessor, bytes4 selector) external;

    /**
     * @dev Returns the maximum allowed time difference for attestation timestamp validation
     * @return Maximum time difference in seconds between attestation time and current block time
     */
    function maxTimeDiff() external view returns (uint64);

    function rootCerts(ProcessorType processorModel) external view returns (bytes32);
    function revokeCertCache(bytes32 _certHash) external;
    function setRootCert(ProcessorType _processorModel, bytes32 _rootCert) external;
    function setZkConfiguration(ZkCoProcessorType zkCoProcessor, ZkCoProcessorConfig memory config) external;
    function checkTrustedIntermediateCerts(ProcessorType[] calldata processorModels, bytes32[][] calldata _reportCerts)
        external
        view
        returns (uint8[] memory);

    function verifyAndAttestWithZKProof(
        bytes calldata output,
        ZkCoProcessorType zkCoprocessor,
        bytes calldata proofBytes
    ) external returns (VerifierJournal memory parsed);

    function verifyAndAttestWithZKProof(
        bytes calldata output,
        ZkCoProcessorType zkCoprocessor,
        bytes32 identifier,
        bytes calldata proofBytes
    ) external returns (VerifierJournal memory parsed);
}
