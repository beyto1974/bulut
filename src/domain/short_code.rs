//! Short, human-readable session code. Lowercase letters and digits without look-alikes.

use std::fmt;

/// Letters without `i l o`, digits without `0 1`: 31 characters.
pub const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShortCode(String);

#[derive(Debug, PartialEq, Eq)]
pub enum ShortCodeError {
    Length { expected: usize, got: usize },
    Character(char),
}

impl fmt::Display for ShortCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { expected, got } => {
                write!(f, "code must be {expected} characters, got {got}")
            }
            Self::Character(c) => write!(f, "character {c:?} is not allowed in a code"),
        }
    }
}

impl std::error::Error for ShortCodeError {}

impl ShortCode {
    /// Validates user input. Input is lowercased first so typed or pasted capitals still work.
    pub fn parse(input: &str, length: usize) -> Result<Self, ShortCodeError> {
        let lower = input.trim().to_ascii_lowercase();
        let got = lower.chars().count();
        if got != length {
            return Err(ShortCodeError::Length {
                expected: length,
                got,
            });
        }
        if let Some(bad) = lower
            .chars()
            .find(|c| !c.is_ascii() || !ALPHABET.contains(&(*c as u8)))
        {
            return Err(ShortCodeError::Character(bad));
        }
        Ok(Self(lower))
    }

    /// Builds a code from already-valid bytes (used by generators).
    pub(crate) fn from_alphabet_indices(indices: impl IntoIterator<Item = usize>) -> Self {
        Self(
            indices
                .into_iter()
                .map(|i| ALPHABET[i % ALPHABET.len()] as char)
                .collect(),
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ShortCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alphabet_has_no_look_alikes() {
        for c in b"01oli" {
            assert!(
                !ALPHABET.contains(c),
                "{} must not be in the alphabet",
                *c as char
            );
        }
        assert_eq!(ALPHABET.len(), 31);
        assert!(ALPHABET
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }

    #[test]
    fn parses_valid_code_and_lowercases() {
        assert_eq!(ShortCode::parse("k7m3q", 5).unwrap().as_str(), "k7m3q");
        assert_eq!(ShortCode::parse(" K7M3Q ", 5).unwrap().as_str(), "k7m3q");
    }

    #[test]
    fn rejects_wrong_length() {
        assert_eq!(
            ShortCode::parse("k7m3", 5),
            Err(ShortCodeError::Length {
                expected: 5,
                got: 4
            })
        );
    }

    #[test]
    fn rejects_look_alikes_and_symbols() {
        assert_eq!(
            ShortCode::parse("k7m3o", 5),
            Err(ShortCodeError::Character('o'))
        );
        assert_eq!(
            ShortCode::parse("k7m31", 5),
            Err(ShortCodeError::Character('1'))
        );
        assert_eq!(
            ShortCode::parse("k7m3-", 5),
            Err(ShortCodeError::Character('-'))
        );
        assert!(ShortCode::parse("k7mé3", 5).is_err());
    }
}
