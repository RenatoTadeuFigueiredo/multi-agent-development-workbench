//! Validated scalar value objects.

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use crate::{CoreError, FailureCategory};

/// A one-based, session-local event sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sequence(NonZeroU64);

impl Sequence {
    /// The first legal sequence.
    pub const FIRST: Self = Self(NonZeroU64::MIN);

    /// Validates a sequence number.
    ///
    /// # Errors
    ///
    /// Returns `invalid_request` when the value is zero.
    pub fn new(value: u64) -> Result<Self, CoreError> {
        NonZeroU64::new(value).map(Self).ok_or_else(|| {
            CoreError::new(
                FailureCategory::InvalidRequest,
                "session sequence must be greater than zero",
            )
        })
    }

    /// Returns the numeric sequence.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Returns the next sequence, or an error on exhaustion.
    ///
    /// # Errors
    ///
    /// Returns `internal` when the sequence space is exhausted.
    pub fn checked_next(self) -> Result<Self, CoreError> {
        self.get()
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .map(Self)
            .ok_or_else(|| {
                CoreError::new(FailureCategory::Internal, "session sequence is exhausted")
            })
    }
}

/// A replay cursor. Zero means replay from the beginning.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Cursor(u64);

impl Cursor {
    /// Creates a cursor after the supplied sequence number.
    #[must_use]
    pub const fn after(sequence: u64) -> Self {
        Self(sequence)
    }

    /// Returns the cursor value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A lowercase, 256-bit hexadecimal content digest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentHash(String);

impl ContentHash {
    /// Validates a lowercase, 64-character hexadecimal digest.
    ///
    /// # Errors
    ///
    /// Returns `invalid_request` when the digest format is invalid.
    pub fn parse(value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        let valid = value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if valid {
            Ok(Self(value))
        } else {
            Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "content hash must be 64 lowercase hexadecimal characters",
            ))
        }
    }

    /// Returns the digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContentHash {
    type Error = CoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ContentHash> for String {
    fn from(value: ContentHash) -> Self {
        value.0
    }
}

macro_rules! identifier {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validates a configuration identifier.
            ///
            /// # Errors
            ///
            /// Returns `invalid_request` when the identifier does not match the schema.
            pub fn parse(value: impl Into<String>) -> Result<Self, CoreError> {
                let value = value.into();
                let mut bytes = value.bytes();
                let first_is_valid = bytes.next().is_some_and(|byte| byte.is_ascii_lowercase());
                let remainder_is_valid = bytes
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
                if first_is_valid && remainder_is_valid && value.len() <= 63 {
                    Ok(Self(value))
                } else {
                    Err(CoreError::new(
                        FailureCategory::InvalidRequest,
                        "identifier must match ^[a-z][a-z0-9-]{0,62}$",
                    ))
                }
            }

            /// Returns the identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = CoreError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

identifier!(RoleId, "Identifies a stable workflow role.");
identifier!(
    ModelAlias,
    "Identifies a provider-independent model mapping."
);
identifier!(ProviderId, "Identifies a provider adapter.");
identifier!(ToolId, "Identifies a centrally registered tool.");
identifier!(
    DataSourceId,
    "Identifies a centrally registered data source."
);
identifier!(WorkflowId, "Identifies a declarative workflow.");

/// Non-empty text whose surrounding whitespace is significant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NonEmptyText(String);

impl NonEmptyText {
    /// Rejects an empty string while preserving all content.
    ///
    /// # Errors
    ///
    /// Returns `invalid_request` when the string is empty.
    pub fn parse(value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        if value.is_empty() {
            Err(CoreError::new(
                FailureCategory::InvalidRequest,
                "text must not be empty",
            ))
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the original text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NonEmptyText {
    type Error = CoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<NonEmptyText> for String {
    fn from(value: NonEmptyText) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{ContentHash, RoleId, Sequence};

    #[test]
    fn rejects_zero_sequence() {
        assert!(Sequence::new(0).is_err());
    }

    #[test]
    fn validates_content_hash_format() {
        assert!(ContentHash::parse("a".repeat(64)).is_ok());
        assert!(ContentHash::parse("A".repeat(64)).is_err());
    }

    proptest! {
        #[test]
        fn accepted_role_ids_round_trip(suffix in "[a-z0-9-]{0,62}") {
            let text = format!("r{suffix}");
            let id = RoleId::parse(text.clone()).expect("generated identifier is valid");
            prop_assert_eq!(id.as_str(), text);
        }
    }
}
