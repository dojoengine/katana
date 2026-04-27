// Certificate Cache Component for AMD SEV-SNP Attestation
// Based on:
// https://github.com/automata-network/amd-sev-snp-attestation-sdk/blob/main/contracts/src/bases/CertCacheBase.sol

#[starknet::component]
pub mod CertCacheComponent {
    use starknet::storage::{Map, StorageMapReadAccess, StorageMapWriteAccess};
    use crate::tee_types::ProcessorType;

    #[storage]
    pub struct Storage {
        /// Mapping of trusted intermediate certificate hashes (excludes root certificate)
        trusted_intermediate_certs: Map<u256, bool>,
        /// Mapping of processor models to their trusted ARK certificate hashes
        root_certs: Map<ProcessorType, u256>,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    pub enum Event {
        TrustedCertInitialized: TrustedCertInitialized,
        RootCertSet: RootCertSet,
        CertRevoked: CertRevoked,
        CertCached: CertCached,
    }

    #[derive(Drop, starknet::Event)]
    pub struct TrustedCertInitialized {
        pub cert_hash: u256,
    }

    #[derive(Drop, starknet::Event)]
    pub struct RootCertSet {
        pub processor_model: ProcessorType,
        pub root_cert: u256,
    }

    #[derive(Drop, starknet::Event)]
    pub struct CertRevoked {
        pub cert_hash: u256,
    }

    #[derive(Drop, starknet::Event)]
    pub struct CertCached {
        pub cert_hash: u256,
    }

    #[starknet::interface]
    pub trait ICertCache<TContractState> {
        fn is_trusted_intermediate_cert(self: @TContractState, cert_hash: u256) -> bool;
        fn get_root_cert(self: @TContractState, processor_model: ProcessorType) -> u256;
        fn check_trusted_intermediate_certs(
            self: @TContractState,
            processor_models: Span<ProcessorType>,
            report_certs: Span<Span<u256>>,
        ) -> Array<u8>;
    }

    #[embeddable_as(CertCacheImpl)]
    impl CertCache<
        TContractState, +HasComponent<TContractState>,
    > of ICertCache<ComponentState<TContractState>> {
        /// Check if a certificate hash is trusted
        fn is_trusted_intermediate_cert(
            self: @ComponentState<TContractState>, cert_hash: u256,
        ) -> bool {
            self.trusted_intermediate_certs.read(cert_hash)
        }

        /// Get the root certificate hash for a processor model
        fn get_root_cert(
            self: @ComponentState<TContractState>, processor_model: ProcessorType,
        ) -> u256 {
            self.root_certs.read(processor_model)
        }

        /// Check the prefix length of trusted certificates in each provided certificate chain
        /// For each certificate chain:
        /// 1. Validates that the first certificate matches the stored root certificate
        /// 2. Counts consecutive trusted certificates starting from the root
        /// 3. Stops counting when an untrusted certificate is encountered
        fn check_trusted_intermediate_certs(
            self: @ComponentState<TContractState>,
            processor_models: Span<ProcessorType>,
            report_certs: Span<Span<u256>>,
        ) -> Array<u8> {
            assert!(report_certs.len() == processor_models.len(), "Array length mismatch");

            let mut results: Array<u8> = array![];

            for (certs, processor_model) in core::iter::zip(report_certs, processor_models) {
                let expected_root_cert = self.root_certs.read(*processor_model);
                assert!(expected_root_cert != 0, "Root certificate not set for processor");
                assert!(
                    *certs.at(0) == expected_root_cert,
                    "First certificate must be root certificate",
                );

                let mut trusted_cert_prefix_len: u8 = 1;
                let mut j: u32 = 1;

                while j < certs.len() {
                    if !self.trusted_intermediate_certs.read(*certs.at(j)) {
                        break;
                    }
                    trusted_cert_prefix_len += 1;
                    j += 1;
                }

                results.append(trusted_cert_prefix_len);
            }

            results
        }
    }

    #[generate_trait]
    pub impl InternalImpl<
        TContractState, +HasComponent<TContractState>,
    > of InternalTrait<TContractState> {
        /// Initialize trusted certificates during contract deployment
        fn initialize_trusted_certs(
            ref self: ComponentState<TContractState>, initialize_trusted_certs: Span<u256>,
        ) {
            for cert_hash in initialize_trusted_certs {
                self.trusted_intermediate_certs.write(*cert_hash, true);
                self.emit(TrustedCertInitialized { cert_hash: *cert_hash });
            }
        }

        /// Set the trusted root certificate hash for a specific processor model
        /// The root certificate serves as the trust anchor for all certificate chain validations.
        /// Different AMD SEV-SNP processors use certificates issued from different root
        /// certificates.
        fn set_root_cert(
            ref self: ComponentState<TContractState>,
            processor_model: ProcessorType,
            root_cert: u256,
        ) {
            self.root_certs.write(processor_model, root_cert);
            self.emit(RootCertSet { processor_model, root_cert });
        }

        /// Revoke a trusted intermediate certificate
        /// This allows revoking compromised intermediate certificates
        /// without affecting the root certificate or other trusted certificates.
        fn revoke_cert_cache(ref self: ComponentState<TContractState>, cert_hash: u256) {
            assert!(
                self.trusted_intermediate_certs.read(cert_hash),
                "Certificate not found in trusted certs",
            );
            self.trusted_intermediate_certs.write(cert_hash, false);
            self.emit(CertRevoked { cert_hash });
        }

        /// Cache newly discovered trusted certificates
        /// This function automatically adds any certificates beyond the trusted length
        /// to the trusted intermediate certificates set. This optimizes future verifications
        /// by expanding the known trusted certificate set based on successful verifications.
        fn cache_new_cert(
            ref self: ComponentState<TContractState>,
            certs: Span<u256>,
            trusted_certs_prefix_len: u32,
        ) {
            let new_trusted_certs = certs
                .slice(trusted_certs_prefix_len, certs.len() - trusted_certs_prefix_len);
            for cert_hash in new_trusted_certs {
                self.trusted_intermediate_certs.write(*cert_hash, true);
                self.emit(CertCached { cert_hash: *cert_hash });
            }
        }
    }
}
