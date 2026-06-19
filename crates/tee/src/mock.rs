use crate::error::TeeError;
use crate::Attester;

/// Mock TEE attester for testing on non-TEE hardware.
///
/// This attester generates deterministic mock quotes that include
/// the user data for verification in tests.
#[derive(Debug, Default, Clone)]
pub struct MockAttester {
    /// Optional custom prefix for mock quotes.
    prefix: Vec<u8>,
}

impl MockAttester {
    /// Create a new mock attester.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a mock attester with a custom prefix.
    pub fn with_prefix(prefix: Vec<u8>) -> Self {
        Self { prefix }
    }
}

impl Attester for MockAttester {
    fn generate_quote(&self, user_data: &[u8; 64]) -> Result<Vec<u8>, TeeError> {
        // Mock quote format:
        // [4 bytes: magic] [prefix] [64 bytes: user_data] [4 bytes: checksum]
        let magic = b"MOCK";
        let mut quote = Vec::with_capacity(4 + self.prefix.len() + 64 + 4);

        quote.extend_from_slice(magic);
        quote.extend_from_slice(&self.prefix);
        quote.extend_from_slice(user_data);

        // Simple checksum for verification
        let checksum: u32 = quote.iter().map(|&b| b as u32).sum();
        quote.extend_from_slice(&checksum.to_le_bytes());

        Ok(quote)
    }

    fn name(&self) -> &'static str {
        "Mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_quote_generation() {
        let attester = MockAttester::new();
        let user_data = [0u8; 64];

        let quote = attester.generate_quote(&user_data).unwrap();

        // Verify magic header
        assert_eq!(&quote[0..4], b"MOCK");
        // Verify user data is included
        assert_eq!(&quote[4..68], &user_data);
    }

    #[test]
    fn test_mock_with_prefix() {
        let prefix = b"TEST".to_vec();
        let attester = MockAttester::with_prefix(prefix.clone());
        let user_data = [1u8; 64];

        let quote = attester.generate_quote(&user_data).unwrap();

        assert_eq!(&quote[0..4], b"MOCK");
        assert_eq!(&quote[4..8], &prefix[..]);
        assert_eq!(&quote[8..72], &user_data);
    }
}
