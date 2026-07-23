---
status: accepted
date: 2026-07-23
deciders: [Renato Figueiredo]
consulted: []
informed: []
---

# Encrypted Local Session Storage

## Context and Problem Statement

Workbench sessions contain prompts, source fragments, model output, tool
results, approvals, and audit events. Same-user file permissions reduce
exposure but do not protect copied disks, backups, SQLite free pages, or WAL
files. Credentials must never enter the event store, while sensitive session
content must remain durable and exportable.

## Decision Drivers

- Session history must survive client and daemon restarts.
- Persistent payloads must not be recoverable without the user's platform key.
- Deletion must remain effective even when SQLite pages have not been erased.
- Key handling must work on macOS and Linux without repository secrets.
- Test suites must remain deterministic and independent from a real key store.
- Portable exports must stay encrypted outside the Workbench state directory.

## Considered Options

1. Application-level envelope encryption for sensitive payloads.
2. Whole-database encryption through a SQLite-compatible encryption extension.
3. Plain SQLite protected only by operating-system file permissions.

## Decision Outcome

Chosen option: **application-level envelope encryption**, because it keeps the
SQLite boundary replaceable, supports cryptographic deletion per session, and
allows non-sensitive ordering metadata to remain queryable without exposing
prompt or provider content.

Each session receives a random 256-bit data-encryption key. Sensitive event and
artifact payloads use XChaCha20-Poly1305 with a fresh 192-bit nonce.
Authenticated associated data binds the ciphertext to the storage schema
version, session identifier, event identifier, sequence, and event kind.

A random 256-bit root key stored as `workbench/storage-root/v1` wraps session
keys. Each wrapped key envelope is stored as
`session/<session-id>/v1`. Both records exist only in macOS Keychain or Linux
Secret Service and are configured as local, non-synchronizing secrets where
the platform supports that distinction. Persistent mode fails with
`key_store_unavailable` when either record cannot be created, unlocked, or
read. Only the deterministic test profile may substitute an in-memory key
store; no plaintext persistent fallback exists.

Key wrapping also uses XChaCha20-Poly1305 with a fresh nonce. Its associated
data binds the envelope schema version, session ID, key ID, and root-key ID.
The implementation must never reuse a nonce under the same key; failure to
obtain cryptographically secure randomness is fatal.

SQLite stores event identifiers, session identifiers, sequence numbers,
timestamps, event kinds, key identifiers, nonces, and ciphertext. It stores
neither wrapped session keys nor plaintext sensitive payloads or credential
values. Key rotation rewraps the platform-stored session-key envelopes under a
new root key without rewriting every payload.

Session deletion first atomically appends an encrypted deletion-intent event
and a non-sensitive deletion journal containing only session, deletion, and
request identifiers. It then removes the wrapped key envelope from the platform
key store and evicts every in-memory copy, making residual ciphertext
inaccessible. It next purges database rows and artifacts and converts the
journal into a non-sensitive deletion tombstone. Recovery completes any
interrupted deletion in the same order. Portable exports decrypt authorized
content in memory and immediately re-encrypt it to explicit age recipients;
neither the root key nor a session key enters the age v1 bundle. Feature 001
does not provide plaintext export or plaintext temporary files. Secret-key and
plaintext buffers are zeroized immediately after their final use.

### Consequences

- Good: copied database and WAL files do not reveal prompt or model content.
- Good: deleting one platform-stored session-key envelope provides
  cryptographic erasure without depending on SQLite page reuse.
- Good: root-key rotation does not require decrypting and re-encrypting every
  event.
- Good: encrypted age exports are independent from the local key store.
- Bad: locked or unavailable platform key stores prevent persistent sessions.
- Bad: losing the root key makes retained sessions unrecoverable.
- Bad: event ordering metadata remains visible to a same-user local observer.
- Bad: encryption adds key migration, corruption recovery, and test vectors to
  the storage contract.

## References

- [RustCrypto XChaCha20-Poly1305 implementation](https://github.com/RustCrypto/AEADs/tree/master/chacha20poly1305)
- [age v1 file format](https://age-encryption.org/v1)
- [Apple Keychain Services](https://developer.apple.com/documentation/security/keychain-services)
- [Secret Service API](https://specifications.freedesktop.org/secret-service/latest/)
