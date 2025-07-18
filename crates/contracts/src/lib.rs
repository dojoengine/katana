use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

pub use katana_contracts_macro::contract;
use katana_primitives::class::{
    ClassHash, CompiledClass, CompiledClassHash, ComputeClassHashError, ContractClass,
    ContractClassCompilationError,
};

pub mod contracts;

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

#[derive(Debug, Clone)]
pub enum ArtifactSource {
    File(PathBuf),
    Embedded(&'static str),
}

/// A unified container for contract artifacts that provides lazy computation of derived data.
#[derive(Debug, Clone)]
pub struct ClassArtifact {
    artifact_source: ArtifactSource,
}

impl ClassArtifact {
    /// Creates a new unified artifact from the given path and metadata.
    ///
    /// This method doesn't validate that the artifact exists at the provided path.
    pub fn new(source: ArtifactSource) -> Self {
        Self { artifact_source: source }
    }

    pub fn embedded(artifact: &'static str) -> Self {
        Self::new(ArtifactSource::Embedded(artifact))
    }

    pub fn file(path: PathBuf) -> Self {
        Self::new(ArtifactSource::File(path))
    }

    /// Gets the Sierra class artifact, loading it lazily on first access.
    ///
    /// # Returns
    /// * `Ok(&ContractClass)` - Reference to the loaded Sierra class
    /// * `Err(ArtifactError)` - If loading or parsing fails
    pub fn class(&self) -> Result<ContractClass, ClassArtifactError> {
        match &self.artifact_source {
            ArtifactSource::Embedded(content) => Ok(ContractClass::from_str(content)?),
            ArtifactSource::File(path) => Ok(ContractClass::from_str(&fs::read_to_string(path)?)?),
        }
    }

    /// Gets the class hash, computing it lazily on first access.
    ///
    /// # Returns
    /// * `Ok(&ClassHash)` - Reference to the computed class hash
    /// * `Err(ArtifactError)` - If computation fails
    pub fn class_hash(&self) -> Result<ClassHash, ClassArtifactError> {
        self.class()?.class_hash().map_err(ClassArtifactError::ClassHash)
    }

    /// Gets the compiled CASM class, compiling it lazily on first access.
    ///
    /// # Returns
    /// * `Ok(&CompiledClass)` - Reference to the compiled CASM class
    /// * `Err(ArtifactError)` - If compilation fails
    pub fn casm(&self) -> Result<CompiledClass, ClassArtifactError> {
        self.class()?.compile().map_err(ClassArtifactError::Compilation)
    }

    /// Gets the compiled class hash, computing it lazily on first access.
    ///
    /// # Returns
    /// * `Ok(&CompiledClassHash)` - Reference to the computed compiled class hash
    /// * `Err(ArtifactError)` - If computation fails
    pub fn casm_hash(&self) -> Result<CompiledClassHash, ClassArtifactError> {
        self.casm()?.class_hash().map_err(ClassArtifactError::ClassHash)
    }
}
