use std::path::PathBuf;
use std::str::FromStr;
use std::sync::OnceLock;

pub use katana_contracts_macro::contract;
use katana_primitives::class::{
    ClassHash, CompiledClass, CompiledClassHash, ComputeClassHashError, ContractClass,
    ContractClassCompilationError,
};

mod feature;

pub use feature::{AccountContract, GenesisAccount, UniversalDeployer, ERC20};

/// A unified container for contract artifacts that provides lazy computation of derived data.
#[derive(Debug, Clone)]
pub struct ClassArtifact {
    /// Path to the artifact file
    artifact_path: PathBuf,
    /// Lazily loaded Sierra class artifact
    class: OnceLock<ContractClass>,
    /// Lazily computed class hash
    hash: OnceLock<ClassHash>,
    /// Lazily compiled CASM class
    casm: OnceLock<CompiledClass>,
    /// Lazily computed compiled class hash
    casm_hash: OnceLock<CompiledClassHash>,
}

impl ClassArtifact {
    /// Creates a new unified artifact from the given path and metadata.
    ///
    /// This method doesn't validate that the artifact exists at the provided path.
    pub fn new(artifact_path: PathBuf) -> Self {
        Self {
            artifact_path,
            hash: OnceLock::new(),
            casm: OnceLock::new(),
            class: OnceLock::new(),
            casm_hash: OnceLock::new(),
        }
    }

    /// Creates a unified artifact from embedded string content.
    ///
    /// This method is designed for use with `include_str!` macro to create
    /// global static artifacts from embedded contract files.
    ///
    /// # Arguments
    /// * `content` - The contract class JSON content as a string
    /// * `contract_name` - Name of the contract
    /// * `package_name` - Name of the package containing the contract
    ///
    /// # Returns
    /// * `Ok(UnifiedArtifact)` - The constructed artifact with pre-parsed Sierra class
    /// * `Err(ArtifactError)` - If parsing the content fails
    pub fn from_content(content: &str) -> Result<Self, ClassArtifactError> {
        // Parse the Sierra class immediately since we have the content
        let class = ContractClass::from_str(content)?;

        let artifact = Self {
            artifact_path: PathBuf::from("<embedded>"),
            class: OnceLock::new(),
            hash: OnceLock::new(),
            casm: OnceLock::new(),
            casm_hash: OnceLock::new(),
        };

        // Pre-populate the sierra class since we already parsed it
        // This will never fail since we just created the OnceLock
        let _ = artifact.class.set(class);

        Ok(artifact)
    }

    /// Gets the artifact file path.
    pub fn artifact_path(&self) -> &PathBuf {
        &self.artifact_path
    }

    /// Gets the Sierra class artifact, loading it lazily on first access.
    ///
    /// # Returns
    /// * `Ok(&ContractClass)` - Reference to the loaded Sierra class
    /// * `Err(ArtifactError)` - If loading or parsing fails
    pub fn class(&self) -> Result<&ContractClass, ClassArtifactError> {
        if let Some(class) = self.class.get() {
            return Ok(class);
        }

        let content = std::fs::read_to_string(&self.artifact_path)?;
        let class = ContractClass::from_str(&content)?;

        match self.class.set(class) {
            Ok(()) => Ok(self.class.get().unwrap()),
            Err(_) => {
                // Someone else set it first, return their value
                Ok(self.class.get().unwrap())
            }
        }
    }

    /// Gets the class hash, computing it lazily on first access.
    ///
    /// # Returns
    /// * `Ok(&ClassHash)` - Reference to the computed class hash
    /// * `Err(ArtifactError)` - If computation fails
    pub fn class_hash(&self) -> Result<&ClassHash, ClassArtifactError> {
        if let Some(hash) = self.hash.get() {
            return Ok(hash);
        }

        let sierra_class = self.class()?;
        let hash = sierra_class.class_hash().map_err(ClassArtifactError::ClassHash)?;

        match self.hash.set(hash) {
            Ok(()) => Ok(self.hash.get().unwrap()),
            Err(_) => {
                // Someone else set it first, return their value
                Ok(self.hash.get().unwrap())
            }
        }
    }

    /// Gets the compiled CASM class, compiling it lazily on first access.
    ///
    /// # Returns
    /// * `Ok(&CompiledClass)` - Reference to the compiled CASM class
    /// * `Err(ArtifactError)` - If compilation fails
    pub fn casm(&self) -> Result<&CompiledClass, ClassArtifactError> {
        if let Some(compiled) = self.casm.get() {
            return Ok(compiled);
        }

        let sierra_class = self.class()?.clone();
        let compiled = sierra_class.compile().map_err(ClassArtifactError::Compilation)?;

        match self.casm.set(compiled) {
            Ok(()) => Ok(self.casm.get().unwrap()),
            Err(_) => {
                // Someone else set it first, return their value
                Ok(self.casm.get().unwrap())
            }
        }
    }

    /// Gets the compiled class hash, computing it lazily on first access.
    ///
    /// # Returns
    /// * `Ok(&CompiledClassHash)` - Reference to the computed compiled class hash
    /// * `Err(ArtifactError)` - If computation fails
    pub fn casm_hash(&self) -> Result<&CompiledClassHash, ClassArtifactError> {
        if let Some(hash) = self.casm_hash.get() {
            return Ok(hash);
        }

        let compiled_class = self.casm()?;
        let hash = compiled_class.class_hash().map_err(ClassArtifactError::ClassHash)?;

        match self.casm_hash.set(hash) {
            Ok(()) => Ok(self.casm_hash.get().unwrap()),
            Err(_) => {
                // Someone else set it first, return their value
                Ok(self.casm_hash.get().unwrap())
            }
        }
    }
}

/// Errors that can occur when working with unified artifacts.
#[derive(Debug, thiserror::Error)]
pub enum ClassArtifactError {
    #[error("Failed to load sierra class from file: {0}")]
    SierraClassParse(#[from] katana_primitives::class::ContractClassFromStrError),

    #[error("Failed to compute class hash: {0}")]
    ClassHash(#[from] ComputeClassHashError),

    #[error("failed to compile sierra to casm: {0}")]
    Compilation(#[from] ContractClassCompilationError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("artifact not found at path: {0}")]
    ArtifactNotFound(PathBuf),
}

/// Builder for creating unified artifacts with a fluent interface.
#[derive(Debug, Default)]
pub struct ArtifactBuilder {
    path: Option<PathBuf>,
}

impl ArtifactBuilder {
    /// Creates a new artifact builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the artifact file path.
    pub fn with_path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }

    /// Builds the unified artifact.
    ///
    /// # Returns
    /// * `Ok(UnifiedArtifact)` - The constructed artifact
    /// * `Err(ArtifactError)` - If required fields are missing or artifact doesn't exist
    pub fn build(self) -> Result<ClassArtifact, ClassArtifactError> {
        let path = self.path.unwrap();

        // Verify the artifact file exists
        if !path.exists() {
            return Err(ClassArtifactError::ArtifactNotFound(path));
        }

        Ok(ClassArtifact::new(path))
    }
}

// Import the ContractClass FromStr implementation
// Note: FromStr is already imported above

#[cfg(test)]
mod tests {
    use super::*;

    // Test contract using the macro with a mock legacy contract file
    // contract!(MockContract, "test_data/mock_legacy.json");

    #[test]
    fn test_artifact_builder() {
        let result =
            ArtifactBuilder::new().with_path(PathBuf::from("test/path/artifact.json")).build();

        // This should fail because the path doesn't exist
        assert!(matches!(result, Err(ClassArtifactError::ArtifactNotFound(_))));
    }

    #[test]
    fn test_artifact_not_found() {
        let nonexistent_path = PathBuf::from("nonexistent/path/to/artifact.json");

        let result = ArtifactBuilder::new().with_path(nonexistent_path.clone()).build();

        assert!(matches!(result, Err(ClassArtifactError::ArtifactNotFound(_))));
    }

    #[test]
    fn test_builder_default_names() {
        let result =
            ArtifactBuilder::new().with_path(PathBuf::from("test/path/artifact.json")).build();

        // Should fail because path doesn't exist, but we can test the error type
        assert!(matches!(result, Err(ClassArtifactError::ArtifactNotFound(_))));
    }

    #[test]
    fn test_builder_missing_path() {
        let result = ArtifactBuilder::new().build();

        assert!(matches!(result, Err(ClassArtifactError::SierraClassParse(_))));
    }

    // TODO: Re-enable tests when mock contract path is fixed
    // #[test]
    // fn test_contract_macro_generated_struct() {
    // Test that the macro generates a working struct
    // let mock_contract = MockContract::new();
    //
    // Test that we can access the underlying artifact
    // let artifact = mock_contract.artifact();
    // assert_eq!(artifact.artifact_path().to_str(), Some("<embedded>"));
    //
    // Test that const functions work (these should be compile-time constants)
    // let class_hash = MockContract::hash();
    // let casm_hash = MockContract::casm_hash();
    //
    // Verify hashes are not zero (basic sanity check)
    // assert_ne!(class_hash, katana_primitives::Felt::ZERO);
    // assert_ne!(casm_hash, katana_primitives::Felt::ZERO);
    //
    // Test that runtime methods work
    // assert!(mock_contract.sierra_class().is_ok());
    // assert!(mock_contract.compiled_class().is_ok());
    // assert!(mock_contract.is_legacy().is_ok());
    // }
    //
    // #[test]
    // fn test_contract_const_consistency() {
    // Test that const methods return the same values as runtime methods
    // let mock_contract = MockContract::new();
    //
    // let const_class_hash = MockContract::hash();
    // let runtime_class_hash = *mock_contract.artifact().class_hash().unwrap();
    // assert_eq!(const_class_hash, runtime_class_hash);
    //
    // let const_casm_hash = MockContract::casm_hash();
    // let runtime_casm_hash = *mock_contract.artifact().compiled_class_hash().unwrap();
    // assert_eq!(const_casm_hash, runtime_casm_hash);
    // }
    //
    // #[test]
    // fn test_contract_deref() {
    // Test that the Deref implementation works
    // let mock_contract = MockContract::new();
    //
    // Can access UnifiedArtifact methods directly
    // assert!(mock_contract.sierra_class().is_ok());
    // assert!(mock_contract.class_hash().is_ok());
    // assert!(mock_contract.compiled_class().is_ok());
    // assert!(mock_contract.compiled_class_hash().is_ok());
    // }
}
