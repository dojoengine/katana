use crate::Felt;

const DEFAULT_SHORT_STRING_MAX_LENGTH: usize = 31;

/// A Cairo short string (31 bytes maximum).
pub type ShortString = FixedAsciiString<DEFAULT_SHORT_STRING_MAX_LENGTH>;

/// Creates a [`ShortString`] from a string literal at compile time.
///
/// This macro is a convenient way to create a `ShortString` from a string literal
/// without having to call `from_static_str` explicitly. The string must be ASCII-only
/// and no longer than 31 bytes.
///
/// # Examples
///
/// ```
/// # use katana_primitives::cairo::{short_string, ShortString};
/// const HELLO: ShortString = short_string!("hello");
/// let world = short_string!("world");
///
/// assert_eq!(HELLO.as_str(), "hello");
/// assert_eq!(world.as_str(), "world");
/// ```
///
/// # Panics
///
/// Panics at compile time if the string is longer than 31 bytes or contains non-ASCII characters.
///
/// ```compile_fail
/// # use katana_primitives::cairo::short_string;
/// // This will fail to compile - string too long
/// const TOO_LONG: ShortString = short_string!("this string is definitely longer than thirty one bytes");
/// ```
///
/// ```compile_fail
/// # use katana_primitives::cairo::short_string;
/// // This will fail to compile - non-ASCII characters
/// const NON_ASCII: ShortString = short_string!("café");
/// ```
#[macro_export]
macro_rules! short_string {
    ($literal:literal) => {
        $crate::cairo::ShortString::from_static_str($literal)
    };
}

impl From<ShortString> for Felt {
    fn from(string: ShortString) -> Self {
        Self::from(&string)
    }
}

impl From<&ShortString> for Felt {
    fn from(string: &ShortString) -> Self {
        Felt::from_bytes_be_slice(string.as_str().as_bytes())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShortStringFromFeltError {
    #[error("Unexpected null terminator in string")]
    UnexpectedNullTerminator,
    #[error("String exceeds maximum length for Cairo short strings")]
    StringTooLong,
    #[error("Non-ASCII character found")]
    NonAsciiCharacter,
    #[error(transparent)]
    FixedAscii(FixedAsciiStringTryFromStrError<DEFAULT_SHORT_STRING_MAX_LENGTH>),
}

impl TryFrom<Felt> for ShortString {
    type Error = ShortStringFromFeltError;

    fn try_from(value: Felt) -> Result<Self, Self::Error> {
        if value == Felt::ZERO {
            return Ok(Self::new());
        }

        let bytes = value.to_bytes_be();

        // First byte must be zero because the string must only be 31 bytes.
        if bytes[0] > 0 {
            return Err(ShortStringFromFeltError::StringTooLong);
        }

        ShortString::try_from(bytes.as_slice()).map_err(ShortStringFromFeltError::FixedAscii)
    }
}

impl TryFrom<&Felt> for ShortString {
    type Error = ShortStringFromFeltError;

    fn try_from(value: &Felt) -> Result<Self, Self::Error> {
        Self::try_from(*value)
    }
}

const STRING_TOO_LONG_ERROR: &str = "String exceeds maximum length";
const INVALID_ASCII_ERROR: &str = "String contains non-ASCII characters";

/// A fixed-capacity ASCII string stored on the stack.
///
/// This is a generic string type that can hold up to `N` ASCII bytes on the stack. It only accepts
/// ASCII characters (0-127).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixedAsciiString<const N: usize> {
    data: [u8; N],
    len: usize,
}

impl<const N: usize> FixedAsciiString<N> {
    /// Creates a new empty fixed string.
    pub const fn new() -> Self {
        Self { data: [0; N], len: 0 }
    }

    /// Creates a [`FixedString`] from a string literal at compile time.
    ///
    /// # Panics
    ///
    /// Panics if the string is longer than `N` bytes or contains non-ASCII characters.
    pub const fn from_static_str(s: &str) -> Self {
        let bytes = s.as_bytes();
        let len = bytes.len();

        if len > N {
            panic!("{}", STRING_TOO_LONG_ERROR);
        }

        // Check that all bytes are ASCII
        let mut i = 0;
        while i < len {
            if bytes[i] > 127 {
                panic!("{}", INVALID_ASCII_ERROR);
            }
            i += 1;
        }

        let mut data = [0; N];
        let mut i = 0;
        while i < len {
            data[i] = bytes[i];
            i += 1;
        }

        Self { data, len }
    }

    pub fn as_str(&self) -> &str {
        // Safety: We only store ASCII characters (0-127), which are valid UTF-8
        unsafe { core::str::from_utf8_unchecked(&self.data[..self.len]) }
    }

    /// Returns the length of the string in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the string is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn push(&mut self, c: char) -> Result<(), ()> {
        // Only accept ASCII characters
        if !c.is_ascii() {
            return Err(());
        }

        let new_len = self.len + 1;
        if new_len > N {
            return Err(());
        }

        self.data[self.len] = c as u8;
        self.len = new_len;
        Ok(())
    }

    #[inline]
    pub fn push_str(&mut self, string: &str) -> Result<(), ()> {
        // Only accept ASCII strings
        if !string.is_ascii() {
            return Err(());
        }

        let new_len = self.len + string.len();
        if new_len > N {
            return Err(());
        }

        self.data[self.len..new_len].copy_from_slice(string.as_bytes());
        self.len = new_len;
        Ok(())
    }
}

impl<const N: usize> Default for FixedAsciiString<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> core::ops::Deref for FixedAsciiString<N> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl<const N: usize> AsRef<str> for FixedAsciiString<N> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const N: usize> core::fmt::Display for FixedAsciiString<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl<const N: usize> From<FixedAsciiString<N>> for String {
    fn from(string: FixedAsciiString<N>) -> Self {
        string.as_str().to_string()
    }
}

#[cfg(feature = "serde")]
impl<const N: usize> serde::Serialize for FixedAsciiString<N> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de, const N: usize> serde::Deserialize<'de> for FixedAsciiString<N> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FixedStringVisitor<const N: usize>;

        impl<'de, const N: usize> serde::de::Visitor<'de> for FixedStringVisitor<N> {
            type Value = FixedAsciiString<N>;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(formatter, "a string with at most {N} ASCII bytes")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                if v.len() > N {
                    return Err(E::custom(format!("String too long: {} bytes > {N} max", v.len())));
                }

                if !v.is_ascii() {
                    return Err(E::custom("String contains non-ASCII characters"));
                }

                let mut fixed = FixedAsciiString::new();
                fixed.push_str(v).map_err(|_| E::custom("Failed to create FixedString"))?;
                Ok(fixed)
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                self.visit_str(&v)
            }

            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                if v.len() > N {
                    return Err(E::custom(format!("Bytes too long: {} bytes > {N} max", v.len())));
                }

                // Check if all bytes are ASCII
                const ASCII_MAX: u8 = 127;
                for &byte in v {
                    if byte > ASCII_MAX {
                        return Err(E::invalid_value(serde::de::Unexpected::Bytes(v), &self));
                    }
                }

                match core::str::from_utf8(v) {
                    Ok(s) => {
                        let mut fixed = FixedAsciiString::new();
                        fixed.push_str(s).map_err(|_| E::custom("Failed to create FixedString"))?;
                        Ok(fixed)
                    }
                    Err(_) => Err(E::invalid_value(serde::de::Unexpected::Bytes(v), &self)),
                }
            }

            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                self.visit_bytes(&v)
            }
        }

        deserializer.deserialize_string(FixedStringVisitor::<N>)
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, const N: usize> arbitrary::Arbitrary<'a> for FixedAsciiString<N> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let max_len = if N > u8::MAX as usize { u8::MAX as usize } else { N };
        let length = u.int_in_range(0..=max_len)?;
        let mut string = Self::new();

        for _ in 0..length {
            let byte = u.int_in_range::<u8>(0..=127)?; // ASCII range
            string.push(byte as char).expect("shouldn't be full");
        }

        Ok(string)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FixedAsciiStringTryFromStrError<const N: usize> {
    #[error("{STRING_TOO_LONG_ERROR}")]
    StringTooLong,
    #[error("{INVALID_ASCII_ERROR}")]
    InvalidAsciiString,
}

impl<const N: usize> core::str::FromStr for FixedAsciiString<N> {
    type Err = FixedAsciiStringTryFromStrError<N>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.is_ascii() {
            return Err(FixedAsciiStringTryFromStrError::InvalidAsciiString);
        }

        if s.len() > N {
            return Err(FixedAsciiStringTryFromStrError::StringTooLong);
        }

        let mut string = Self::new();
        string.push_str(s).expect("length already checked");

        Ok(string)
    }
}

impl<const N: usize> core::convert::TryFrom<&[u8]> for FixedAsciiString<N> {
    type Error = FixedAsciiStringTryFromStrError<N>;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.len() > N {
            return Err(FixedAsciiStringTryFromStrError::StringTooLong);
        }

        let data = core::str::from_utf8(bytes)
            .map_err(|_| FixedAsciiStringTryFromStrError::InvalidAsciiString)?;

        if !data.is_ascii() {
            return Err(FixedAsciiStringTryFromStrError::InvalidAsciiString);
        }

        let mut string = Self::new();
        string.push_str(data).unwrap();

        Ok(string)
    }
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use assert_matches::assert_matches;

    use super::{FixedAsciiString, ShortString};
    use crate::cairo::{FixedAsciiStringTryFromStrError, ShortStringFromFeltError};
    use crate::{short_string, Felt};

    #[test]
    fn new_short_string_is_empty() {
        let s = ShortString::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(s.as_str(), "");
    }

    #[test]
    fn from_static_str_works() {
        const SHORT: ShortString = ShortString::from_static_str("hello");
        assert_eq!(SHORT.as_str(), "hello");
        assert_eq!(SHORT.len(), 5);

        const MAX_LEN: ShortString =
            ShortString::from_static_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"); // 31 'a's
        assert_eq!(MAX_LEN.len(), 31);
    }

    #[test]
    #[should_panic(expected = "String exceeds maximum length")]
    fn from_static_str_too_long() {
        let long_str = "this string is definitely longer than thirty one bytes";
        let _ = ShortString::from_static_str(long_str);
    }

    #[test]
    #[should_panic(expected = "String contains non-ASCII characters")]
    fn from_static_str_non_ascii() {
        let non_ascii = "café";
        let _ = ShortString::from_static_str(non_ascii);
    }

    #[test]
    fn try_from_str() {
        let s = ShortString::from_str("hello").unwrap();
        assert_eq!(s.as_str(), "hello");
        assert_eq!(s.len(), 5);

        let s = "a".repeat(31);
        let short = ShortString::from_str(&s).unwrap();
        assert_eq!(short.len(), 31);

        let long_str = "a".repeat(32);
        assert!(ShortString::from_str(long_str.as_str()).is_err());
    }

    #[test]
    fn round_trip_felt() {
        let original = ShortString::from_str("abc").unwrap();
        let felt = Felt::from(original.clone());
        let converted = ShortString::try_from(felt).unwrap();
        assert_eq!(original, converted);
    }

    #[test]
    fn felt_with_non_zero_first_byte() {
        // Create felt with non-zero first byte
        let mut bytes = [0u8; 32];
        bytes[0] = 1;
        let felt = Felt::from_bytes_be(&bytes);
        assert_matches!(ShortString::try_from(felt), Err(ShortStringFromFeltError::StringTooLong));
    }

    #[test]
    fn felt_with_valid_string() {
        let mut bytes = [0u8; 32];
        bytes[27..32].copy_from_slice(b"hello");
        let felt = Felt::from_bytes_be(&bytes);
        let s = ShortString::try_from(felt).unwrap();
        assert_eq!(s.as_str(), "hello");
    }

    #[test]
    fn felt_with_trailing_non_zero() {
        let mut bytes = [0u8; 32];
        bytes[31] = b'a';
        let felt = Felt::from_bytes_be(&bytes);
        let s = ShortString::try_from(felt).unwrap();
        assert_eq!(s.as_str(), "a");
    }

    #[test]
    fn felt_with_max_length() {
        let mut bytes = [0u8; 32];
        let s = "a".repeat(31);
        bytes[1..].copy_from_slice(s.as_bytes());
        let felt = Felt::from_bytes_be(&bytes);
        let result = ShortString::try_from(felt).unwrap();
        assert_eq!(result.len(), 31);
        assert_eq!(result.as_str(), s);
    }

    #[test]
    fn felt_zero() {
        let s = ShortString::try_from(Felt::ZERO).unwrap();
        assert!(s.is_empty());
    }

    #[rstest::rstest]
    #[case({
        let mut bytes = [0u8; 32];
        bytes[1] = b'a';
        bytes[2] = 0;
        bytes[3] = b'b';
        bytes
    })]
    #[case({
        let mut bytes = [0u8; 32];
        bytes[1] = b'a';
        bytes[2] = 0;
        bytes
    })]
    fn test_felt_with_null(#[case] bytes: [u8; 32]) {
        let felt = Felt::from_bytes_be(&bytes);
        assert!(matches!(
            ShortString::try_from(felt),
            Err(ShortStringFromFeltError::UnexpectedNullTerminator)
        ));
    }

    #[test]
    fn try_from_non_ascii_str() {
        assert_matches!(
            ShortString::from_str("café"),
            Err(FixedAsciiStringTryFromStrError::InvalidAsciiString)
        );
    }

    #[cfg(feature = "arbitrary")]
    #[test]
    fn test_arbitrary_short_string() {
        use arbitrary::{Arbitrary, Unstructured};

        let data = vec![0u8; 128];
        let mut u = Unstructured::new(&data);

        for _ in 0..100 {
            let s = ShortString::arbitrary(&mut u).unwrap();
            assert!(s.len() <= 31);
            assert!(String::from(s).into_bytes().into_iter().all(|b| b <= 127));
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_serde_round_trip() {
        let original = ShortString::from_str("hello world").unwrap();

        // Test JSON serialization/deserialization
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, r#""hello world""#);

        let deserialized: ShortString = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);

        // Test different string lengths
        let empty = ShortString::new();
        let json = serde_json::to_string(&empty).unwrap();
        let deserialized: ShortString = serde_json::from_str(&json).unwrap();
        assert_eq!(empty, deserialized);

        // Test max length string
        let max_str = "a".repeat(31);
        let max_short = ShortString::from_str(&max_str).unwrap();
        let json = serde_json::to_string(&max_short).unwrap();
        let deserialized: ShortString = serde_json::from_str(&json).unwrap();
        assert_eq!(max_short, deserialized);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_serde_error_cases() {
        // Test string too long
        let long_str = format!(r#""{}""#, "a".repeat(32));
        let result: Result<ShortString, _> = serde_json::from_str(&long_str);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too long"));

        // Test non-ASCII string
        let non_ascii = r#""café""#;
        let result: Result<ShortString, _> = serde_json::from_str(non_ascii);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("non-ASCII"));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_generic_fixed_string_serde() {
        // Test with different sizes
        type FixedString10 = FixedAsciiString<10>;
        type FixedString100 = FixedAsciiString<100>;

        let short: FixedString10 = serde_json::from_str(r#""hello""#).unwrap();
        assert_eq!(short.as_str(), "hello");

        let longer: FixedString100 =
            serde_json::from_str(r#""this is a longer string that fits in 100 bytes""#).unwrap();
        assert_eq!(longer.as_str(), "this is a longer string that fits in 100 bytes");

        // Test size limit enforcement
        let too_long_for_10 = r#""this string is definitely longer than 10 bytes""#;
        let result: Result<FixedString10, _> = serde_json::from_str(too_long_for_10);
        assert!(result.is_err());
    }

    #[test]
    fn test_ascii_only_enforcement() {
        let mut s = ShortString::new();

        // ASCII characters should work
        assert!(s.push('A').is_ok());
        assert!(s.push('z').is_ok());
        assert!(s.push('1').is_ok());
        assert!(s.push('!').is_ok());
        assert_eq!(s.as_str(), "Az1!");

        // Non-ASCII characters should fail
        assert!(s.push('é').is_err());
        assert!(s.push('🦀').is_err());
        assert!(s.push('中').is_err());

        // String should remain unchanged after failed pushes
        assert_eq!(s.as_str(), "Az1!");
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn test_ascii_only_push_str() {
        let mut s = ShortString::new();

        // ASCII strings should work
        assert!(s.push_str("hello").is_ok());
        assert_eq!(s.as_str(), "hello");

        assert!(s.push_str(" world").is_ok());
        assert_eq!(s.as_str(), "hello world");

        // Non-ASCII strings should fail
        let original_len = s.len();
        assert!(s.push_str(" café").is_err());
        assert!(s.push_str(" 🦀").is_err());
        assert!(s.push_str(" 中文").is_err());

        // String should remain unchanged after failed pushes
        assert_eq!(s.as_str(), "hello world");
        assert_eq!(s.len(), original_len);
    }

    #[test]
    fn test_fixed_string_different_sizes() {
        type TinyString = FixedAsciiString<5>;
        type BigString = FixedAsciiString<1000>;

        let mut tiny = TinyString::new();
        assert!(tiny.push_str("hello").is_ok());
        assert!(tiny.push_str("!").is_err()); // Should fail, exceeds capacity

        let mut big = BigString::new();
        let long_ascii = "a".repeat(500);
        assert!(big.push_str(&long_ascii).is_ok());
        assert_eq!(big.len(), 500);
    }

    #[test]
    fn test_short_string_macro() {
        // Test basic usage
        const HELLO: ShortString = short_string!("hello");
        assert_eq!(HELLO.as_str(), "hello");
        assert_eq!(HELLO.len(), 5);

        // Test runtime usage
        let world = short_string!("world");
        assert_eq!(world.as_str(), "world");
        assert_eq!(world.len(), 5);

        // Test empty string
        const EMPTY: ShortString = short_string!("");
        assert!(EMPTY.is_empty());
        assert_eq!(EMPTY.len(), 0);

        // Test different lengths
        const ONE_CHAR: ShortString = short_string!("a");
        assert_eq!(ONE_CHAR.len(), 1);

        const MEDIUM: ShortString = short_string!("hello world");
        assert_eq!(MEDIUM.len(), 11);

        // Test max length (31 chars)
        const MAX_LEN: ShortString = short_string!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(MAX_LEN.len(), 31);
        assert_eq!(MAX_LEN.as_str(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

        // Test ASCII characters
        const ASCII_CHARS: ShortString = short_string!("Hello123!@#$%^&*()");
        assert_eq!(ASCII_CHARS.as_str(), "Hello123!@#$%^&*()");
    }

    #[test]
    fn test_short_string_macro_compile_time() {
        // Test that the macro works at compile time
        const COMPILE_TIME: ShortString = short_string!("compile time");

        // This should be evaluated at compile time
        assert_eq!(COMPILE_TIME.as_str(), "compile time");
        assert_eq!(COMPILE_TIME.len(), 12);

        // Test in static context
        static STATIC_STR: ShortString = short_string!("static");
        assert_eq!(STATIC_STR.as_str(), "static");
    }

    #[test]
    fn test_short_string_macro_vs_from_static_str() {
        // Verify that the macro produces the same result as from_static_str
        const VIA_MACRO: ShortString = short_string!("test");
        const VIA_FUNCTION: ShortString = ShortString::from_static_str("test");

        assert_eq!(VIA_MACRO, VIA_FUNCTION);
        assert_eq!(VIA_MACRO.as_str(), VIA_FUNCTION.as_str());
        assert_eq!(VIA_MACRO.len(), VIA_FUNCTION.len());
    }
}
