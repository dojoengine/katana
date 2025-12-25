use crate::Felt;

/// A Cairo short string.
///
/// This is a stack-allocated string type that can hold up to 31 ASCII bytes,
/// which is the maximum length for a Cairo short string.
///
/// It supports const construction via [`ShortString::from_ascii`].
#[derive(Clone, PartialEq, Eq, Hash, Default, Copy)]
pub struct ShortString {
    data: [u8; 31],
    len: u8,
}

impl ShortString {
    /// Creates a new empty short string.
    pub const fn new() -> Self {
        Self { data: [0; 31], len: 0 }
    }

    /// Creates a new short string from an ASCII string literal at compile time.
    ///
    /// # Panics
    ///
    /// Panics at compile time if the string is longer than 31 bytes or contains
    /// non-ASCII characters.
    ///
    /// # Examples
    ///
    /// ```
    /// use katana_primitives::cairo::ShortString;
    ///
    /// const HELLO: ShortString = ShortString::from_ascii("hello");
    /// assert_eq!(HELLO.as_str(), "hello");
    /// ```
    pub const fn from_ascii(s: &str) -> Self {
        let bytes = s.as_bytes();
        let len = bytes.len();

        assert!(len <= 31, "string is too long to be a Cairo short string");

        let mut data = [0u8; 31];
        let mut i = 0;
        while i < len {
            let b = bytes[i];
            assert!(b.is_ascii(), "invalid ASCII character in string");
            data[i] = b;
            i += 1;
        }

        Self { data, len: len as u8 }
    }

    pub const fn as_str(&self) -> &str {
        // SAFETY: We only store valid ASCII bytes, which are valid UTF-8
        unsafe { core::str::from_utf8_unchecked(self.as_bytes()) }
    }

    /// Returns the bytes of the short string as a slice.
    pub const fn as_bytes(&self) -> &[u8] {
        // Use a manual slice since `&self.data[..self.len as usize]` is not const-stable
        unsafe { core::slice::from_raw_parts(self.data.as_ptr(), self.len as usize) }
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    fn push(&mut self, c: char) -> Result<(), ShortStringError> {
        if !c.is_ascii() {
            return Err(ShortStringError::InvalidAscii);
        }

        if self.len >= 31 {
            return Err(ShortStringError::ExceedsCapacity);
        }

        self.data[self.len as usize] = c as u8;
        self.len += 1;

        Ok(())
    }

    #[inline]
    fn push_str(&mut self, string: &str) -> Result<(), ShortStringError> {
        let bytes = string.as_bytes();

        if bytes.len() + self.len as usize > 31 {
            return Err(ShortStringError::ExceedsCapacity);
        }

        for &b in bytes {
            if !b.is_ascii() {
                return Err(ShortStringError::InvalidAscii);
            }

            self.data[self.len as usize] = b;
            self.len += 1;
        }

        Ok(())
    }
}

/// Error returned when constructing or modifying a [`ShortString`] fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShortStringError {
    #[error("string is longer than 31 bytes")]
    ExceedsCapacity,

    #[error("invalid ASCII character")]
    InvalidAscii,

    #[error("unexpected null terminator")]
    UnexpectedNullTerminator,
}

impl core::ops::Deref for ShortString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for ShortString {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<Felt> for ShortString {
    fn eq(&self, other: &Felt) -> bool {
        Felt::from(self) == *other
    }
}

impl core::fmt::Debug for ShortString {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("ShortString").field(&self.as_str()).finish()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ShortString {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ShortString {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ShortStringVisitor;

        impl serde::de::Visitor<'_> for ShortStringVisitor {
            type Value = ShortString;

            fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter.write_str("a string up to 31 ASCII characters")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.parse().map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(ShortStringVisitor)
    }
}

impl core::str::FromStr for ShortString {
    type Err = ShortStringError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.is_ascii() {
            return Err(ShortStringError::InvalidAscii);
        }

        if s.len() > 31 {
            return Err(ShortStringError::ExceedsCapacity);
        }

        let mut string = Self::new();
        string.push_str(s).expect("qed; length already checked");

        Ok(string)
    }
}

impl From<ShortString> for String {
    fn from(string: ShortString) -> Self {
        string.as_str().to_string()
    }
}

impl From<ShortString> for Felt {
    fn from(string: ShortString) -> Self {
        Self::from(&string)
    }
}

impl From<&ShortString> for Felt {
    fn from(string: &ShortString) -> Self {
        Felt::from_bytes_be_slice(string.as_bytes())
    }
}

impl TryFrom<Felt> for ShortString {
    type Error = ShortStringError;

    fn try_from(value: Felt) -> Result<Self, Self::Error> {
        if value == Felt::ZERO {
            return Ok(Self::new());
        }

        let bytes = value.to_bytes_be();

        // First byte must be zero because the string must only be 31 bytes.
        if bytes[0] > 0 {
            return Err(ShortStringError::ExceedsCapacity);
        }

        let mut string = ShortString::new();

        for byte in bytes {
            if byte == 0u8 {
                if !string.is_empty() {
                    return Err(ShortStringError::UnexpectedNullTerminator);
                }
            } else if byte.is_ascii() {
                string.push(byte as char).expect("qed; should fit");
            } else {
                return Err(ShortStringError::InvalidAscii);
            }
        }

        Ok(string)
    }
}

impl TryFrom<&Felt> for ShortString {
    type Error = ShortStringError;

    fn try_from(value: &Felt) -> Result<Self, Self::Error> {
        Self::try_from(*value)
    }
}

impl core::fmt::Display for ShortString {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for ShortString {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let length: u8 = u.int_in_range(0..=31)?;
        let mut data = [0u8; 31];

        for item in data.iter_mut().take(length as usize) {
            // ASCII printable range (32-126) to avoid control characters
            *item = u.int_in_range(32..=126)?;
        }

        Ok(Self { data, len: length })
    }
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use assert_matches::assert_matches;

    use super::ShortString;
    use crate::cairo::ShortStringError;
    use crate::Felt;

    #[test]
    fn new_short_string_is_empty() {
        let s = ShortString::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(s.as_str(), "");
    }

    #[test]
    fn const_from_ascii() {
        const HELLO: ShortString = ShortString::from_ascii("hello");
        assert_eq!(HELLO.as_str(), "hello");
        assert_eq!(HELLO.len(), 5);

        const EMPTY: ShortString = ShortString::from_ascii("");
        assert!(EMPTY.is_empty());

        const MAX_LEN: ShortString = ShortString::from_ascii("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(MAX_LEN.len(), 31);
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
    fn eq_felt() {
        let s = ShortString::from_str("hello").unwrap();
        let felt = Felt::from(&s);

        assert!(s == felt);
        assert!(s != Felt::from(123u64));
    }

    #[test]
    fn felt_with_non_zero_first_byte() {
        // Create felt with non-zero first byte
        let mut bytes = [0u8; 32];
        bytes[0] = 1;
        let felt = Felt::from_bytes_be(&bytes);
        assert_matches!(ShortString::try_from(felt), Err(ShortStringError::ExceedsCapacity));
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
            Err(ShortStringError::UnexpectedNullTerminator)
        ));
    }

    #[test]
    fn try_from_non_ascii_str() {
        assert_matches!(ShortString::from_str("café"), Err(ShortStringError::InvalidAscii));
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
            // Verify all characters are ASCII printable (32-126)
            assert!(s.as_bytes().iter().all(|&b| (32..=126).contains(&b)));
        }
    }
}
