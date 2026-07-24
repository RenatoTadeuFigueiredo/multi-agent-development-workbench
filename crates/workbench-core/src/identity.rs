//! Strongly typed domain identifiers.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_identifier {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a time-ordered `UUIDv7` identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Wraps an already validated UUID.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the underlying UUID.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

uuid_identifier!(SessionId, "Identifies one durable orchestration session.");
uuid_identifier!(EventId, "Identifies one immutable domain event.");
uuid_identifier!(RequestId, "Identifies one idempotent client request.");
uuid_identifier!(InputId, "Identifies one durable user input.");
uuid_identifier!(AttemptId, "Identifies one external side-effect attempt.");
uuid_identifier!(ApprovalId, "Identifies one pending approval.");
uuid_identifier!(ControlId, "Identifies one durable session control.");
uuid_identifier!(CorrelationId, "Correlates one redacted failure.");
uuid_identifier!(ExportId, "Identifies one encrypted portable export.");
uuid_identifier!(DeletionId, "Identifies one cryptographic deletion.");

#[cfg(test)]
mod tests {
    use super::SessionId;

    #[test]
    fn generated_ids_are_uuid_v7() {
        assert_eq!(SessionId::new().as_uuid().get_version_num(), 7);
    }
}
