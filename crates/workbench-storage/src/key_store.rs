use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{
    AssociatedData, EncryptedPayload, SecretKey, StorageError,
    crypto::{KEY_BYTES, NONCE_BYTES},
};

const PLATFORM_SERVICE: &str = "multi-agent-development-workbench";
const PLATFORM_CATALOG_PREFIX: &str = "workbench/key-catalog/v2";
const KEY_NAMESPACE_PREFIX: &str = "workbench/storage";

/// Minimal secret-store contract required for envelope encryption.
pub trait KeyStore: Send + Sync {
    fn put(&self, id: &str, secret: &[u8]) -> Result<(), StorageError>;
    fn get(&self, id: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError>;
    fn delete(&self, id: &str) -> Result<(), StorageError>;
    fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError>;
}

trait CatalogPutOps {
    fn get_secret(&mut self, id: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError>;
    fn set_secret(&mut self, id: &str, secret: &[u8]) -> Result<(), StorageError>;
    fn delete_secret(&mut self, id: &str) -> Result<(), StorageError>;
    fn read_catalog(&mut self, catalog_id: &str) -> Result<BTreeSet<String>, StorageError>;
    fn write_catalog(
        &mut self,
        catalog_id: &str,
        catalog: &BTreeSet<String>,
    ) -> Result<(), StorageError>;
}

fn cataloged_put(
    operations: &mut impl CatalogPutOps,
    id: &str,
    secret: &[u8],
    catalog_id: &str,
) -> Result<(), StorageError> {
    let previous = operations.get_secret(id)?;
    let mut catalog = operations.read_catalog(catalog_id)?;
    operations.set_secret(id, secret)?;
    catalog.insert(id.to_owned());
    if let Err(error) = operations.write_catalog(catalog_id, &catalog) {
        if let Some(previous) = previous.as_deref() {
            operations.set_secret(id, previous)?;
        } else {
            operations.delete_secret(id)?;
        }
        return Err(error);
    }
    Ok(())
}

/// Deterministic test key store. It is never selected by persistent production setup.
#[derive(Clone, Default)]
pub struct MemoryKeyStore {
    entries: Arc<Mutex<HashMap<String, Zeroizing<Vec<u8>>>>>,
    available: Arc<Mutex<bool>>,
}

impl MemoryKeyStore {
    pub fn new() -> Self {
        Self {
            entries: Arc::default(),
            available: Arc::new(Mutex::new(true)),
        }
    }

    pub fn set_available(&self, available: bool) {
        *self.available.lock().expect("availability mutex poisoned") = available;
    }

    fn require_available(&self) -> Result<(), StorageError> {
        if *self.available.lock().expect("availability mutex poisoned") {
            Ok(())
        } else {
            Err(StorageError::KeyStoreUnavailable(None))
        }
    }
}

impl KeyStore for MemoryKeyStore {
    fn put(&self, id: &str, secret: &[u8]) -> Result<(), StorageError> {
        self.require_available()?;
        self.entries
            .lock()
            .expect("key-store mutex poisoned")
            .insert(id.to_owned(), Zeroizing::new(secret.to_vec()));
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError> {
        self.require_available()?;
        Ok(self
            .entries
            .lock()
            .expect("key-store mutex poisoned")
            .get(id)
            .cloned())
    }

    fn delete(&self, id: &str) -> Result<(), StorageError> {
        self.require_available()?;
        self.entries
            .lock()
            .expect("key-store mutex poisoned")
            .remove(id);
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        self.require_available()?;
        let mut ids: Vec<_> = self
            .entries
            .lock()
            .expect("key-store mutex poisoned")
            .keys()
            .filter(|id| id.starts_with(prefix))
            .cloned()
            .collect();
        ids.sort();
        Ok(ids)
    }
}

/// macOS Keychain or Linux Secret Service adapter selected by `keyring`.
#[derive(Debug, Clone, Default)]
pub struct PlatformKeyStore;

impl PlatformKeyStore {
    pub fn new() -> Self {
        Self
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn entry(id: &str) -> Result<keyring::Entry, StorageError> {
        keyring::Entry::new(PLATFORM_SERVICE, id)
            .map_err(|error| StorageError::KeyStoreUnavailable(Some(Box::new(error))))
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn catalog(catalog_id: &str) -> Result<BTreeSet<String>, StorageError> {
        match Self::entry(catalog_id)?.get_secret() {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| StorageError::KeyStoreUnavailable(Some(Box::new(error)))),
            Err(keyring::Error::NoEntry) => Ok(BTreeSet::new()),
            Err(error) => Err(StorageError::KeyStoreUnavailable(Some(Box::new(error)))),
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn write_catalog(catalog_id: &str, catalog: &BTreeSet<String>) -> Result<(), StorageError> {
        let encoded = Zeroizing::new(serde_json::to_vec(catalog)?);
        Self::entry(catalog_id)?
            .set_secret(&encoded)
            .map_err(|error| StorageError::KeyStoreUnavailable(Some(Box::new(error))))
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct PlatformCatalogPutOps;

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl CatalogPutOps for PlatformCatalogPutOps {
    fn get_secret(&mut self, id: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError> {
        match PlatformKeyStore::entry(id)?.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(StorageError::KeyStoreUnavailable(Some(Box::new(error)))),
        }
    }

    fn set_secret(&mut self, id: &str, secret: &[u8]) -> Result<(), StorageError> {
        PlatformKeyStore::entry(id)?
            .set_secret(secret)
            .map_err(|error| StorageError::KeyStoreUnavailable(Some(Box::new(error))))
    }

    fn delete_secret(&mut self, id: &str) -> Result<(), StorageError> {
        match PlatformKeyStore::entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(StorageError::KeyStoreUnavailable(Some(Box::new(error)))),
        }
    }

    fn read_catalog(&mut self, catalog_id: &str) -> Result<BTreeSet<String>, StorageError> {
        PlatformKeyStore::catalog(catalog_id)
    }

    fn write_catalog(
        &mut self,
        catalog_id: &str,
        catalog: &BTreeSet<String>,
    ) -> Result<(), StorageError> {
        PlatformKeyStore::write_catalog(catalog_id, catalog)
    }
}

impl KeyStore for PlatformKeyStore {
    fn put(&self, id: &str, secret: &[u8]) -> Result<(), StorageError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let catalog_id = platform_catalog_id(id)?;
            cataloged_put(&mut PlatformCatalogPutOps, id, secret, &catalog_id)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _unused = (id, secret);
            Err(StorageError::KeyStoreUnavailable(None))
        }
    }

    fn get(&self, id: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            match Self::entry(id)?.get_secret() {
                Ok(secret) => Ok(Some(Zeroizing::new(secret))),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(StorageError::KeyStoreUnavailable(Some(Box::new(error)))),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _unused = id;
            Err(StorageError::KeyStoreUnavailable(None))
        }
    }

    fn delete(&self, id: &str) -> Result<(), StorageError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let catalog_id = platform_catalog_id(id)?;
            match Self::entry(id)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(error) => {
                    return Err(StorageError::KeyStoreUnavailable(Some(Box::new(error))));
                }
            }
            let mut catalog = Self::catalog(&catalog_id)?;
            catalog.remove(id);
            if catalog.is_empty() {
                match Self::entry(&catalog_id)?.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(error) => Err(StorageError::KeyStoreUnavailable(Some(Box::new(error)))),
                }
            } else {
                Self::write_catalog(&catalog_id, &catalog)
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _unused = id;
            Err(StorageError::KeyStoreUnavailable(None))
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            Ok(Self::catalog(&platform_catalog_id(prefix)?)?
                .into_iter()
                .filter(|id| id.starts_with(prefix))
                .collect())
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _unused = prefix;
            Err(StorageError::KeyStoreUnavailable(None))
        }
    }
}

fn platform_catalog_id(key_or_prefix: &str) -> Result<String, StorageError> {
    let mut components = key_or_prefix.split('/');
    if components.next() == Some("workbench")
        && components.next() == Some("storage")
        && let (Some(storage_id), Some(location_id)) = (components.next(), components.next())
        && !storage_id.is_empty()
        && !location_id.is_empty()
    {
        return Ok(format!(
            "{PLATFORM_CATALOG_PREFIX}/{storage_id}/{location_id}"
        ));
    }
    Err(StorageError::InvalidInput(
        "key-store access requires a complete database namespace",
    ))
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionKeyEnvelope {
    schema_version: u32,
    session_id: Uuid,
    key_id: String,
    root_key_id: String,
    algorithm: String,
    nonce: String,
    wrapped_key: String,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct RootKeyRecord {
    schema_version: u32,
    current: String,
    previous: Option<String>,
}

/// Creates, unwraps, caches, rotates, and destroys per-session data keys.
pub struct KeyManager<K> {
    store: K,
    root_key_id: String,
    session_key_prefix: String,
    cache: Mutex<HashMap<Uuid, SecretKey>>,
}

impl<K: KeyStore> KeyManager<K> {
    pub fn for_storage(store: K, storage_id: Uuid, location_scope: &[u8]) -> Self {
        let location_id = blake3::hash(location_scope).to_hex();
        let namespace = format!("{KEY_NAMESPACE_PREFIX}/{storage_id}/{location_id}");
        Self {
            store,
            root_key_id: format!("{namespace}/root/v1"),
            session_key_prefix: format!("{namespace}/session/"),
            cache: Mutex::default(),
        }
    }

    pub fn create_session_key(&self, session_id: Uuid) -> Result<String, StorageError> {
        let key_id = self.session_key_id(session_id);
        if self.store.get(&key_id)?.is_some() {
            return Err(StorageError::InvalidInput("session key already exists"));
        }
        let root_key = self.load_or_create_root_keys()?.0;
        let session_key = SecretKey::generate();
        let wrapped = EncryptedPayload::encrypt(
            &root_key,
            session_key.expose(),
            &envelope_aad(session_id, &key_id, &self.root_key_id),
        )?;
        let envelope = SessionKeyEnvelope {
            schema_version: 1,
            session_id,
            key_id: key_id.clone(),
            root_key_id: self.root_key_id.clone(),
            algorithm: "xchacha20-poly1305".to_owned(),
            nonce: STANDARD.encode(&wrapped.nonce),
            wrapped_key: STANDARD.encode(&wrapped.ciphertext),
        };
        let encoded = Zeroizing::new(serde_json::to_vec(&envelope)?);
        self.store.put(&key_id, &encoded)?;
        self.cache
            .lock()
            .expect("key-cache mutex poisoned")
            .insert(session_id, session_key);
        Ok(key_id)
    }

    pub fn session_key(&self, session_id: Uuid) -> Result<SecretKey, StorageError> {
        if let Some(key) = self
            .cache
            .lock()
            .expect("key-cache mutex poisoned")
            .get(&session_id)
            .cloned()
        {
            return Ok(key);
        }
        let key_id = self.session_key_id(session_id);
        let encoded = self
            .store
            .get(&key_id)?
            .ok_or(StorageError::KeyStoreUnavailable(None))?;
        let envelope: SessionKeyEnvelope = serde_json::from_slice(&encoded)?;
        validate_envelope(&envelope, session_id, &key_id, &self.root_key_id)?;
        let encrypted = EncryptedPayload {
            nonce: STANDARD
                .decode(&envelope.nonce)
                .map_err(|_| StorageError::AuthenticationFailed)?,
            ciphertext: STANDARD
                .decode(&envelope.wrapped_key)
                .map_err(|_| StorageError::AuthenticationFailed)?,
        };
        if encrypted.nonce.len() != NONCE_BYTES {
            return Err(StorageError::AuthenticationFailed);
        }
        let (current_root, previous_root) = self.root_keys()?;
        let plaintext = decrypt_with_root_candidates(
            &encrypted,
            &current_root,
            previous_root.as_ref(),
            &envelope_aad(session_id, &key_id, &self.root_key_id),
        )?;
        let session_key = SecretKey::from_slice(&plaintext)?;
        self.cache
            .lock()
            .expect("key-cache mutex poisoned")
            .insert(session_id, session_key.clone());
        Ok(session_key)
    }

    pub fn destroy_session_key(&self, session_id: Uuid) -> Result<(), StorageError> {
        let result = self.store.delete(&self.session_key_id(session_id));
        self.cache
            .lock()
            .expect("key-cache mutex poisoned")
            .remove(&session_id);
        result
    }

    pub fn session_key_ids(&self) -> Result<Vec<String>, StorageError> {
        self.store.list(&self.session_key_prefix)
    }

    pub(crate) fn session_key_id(&self, session_id: Uuid) -> String {
        format!("{}{session_id}/v1", self.session_key_prefix)
    }

    pub(crate) fn owns_session_key(&self, session_id: Uuid, key_id: &str) -> bool {
        self.session_key_id(session_id) == key_id
    }

    pub fn rotate_root_key(&self) -> Result<(), StorageError> {
        let (old_root, previous_root) = self.load_or_create_root_keys()?;
        if previous_root.is_some() {
            self.rewrap_all_session_keys(&old_root, previous_root.as_ref())?;
            self.write_root_record(&old_root, None)?;
        }
        let new_root = SecretKey::generate();
        self.write_root_record(&new_root, Some(&old_root))?;
        self.rewrap_all_session_keys(&new_root, Some(&old_root))?;
        self.write_root_record(&new_root, None)
    }

    fn rewrap_all_session_keys(
        &self,
        target_root: &SecretKey,
        fallback_root: Option<&SecretKey>,
    ) -> Result<(), StorageError> {
        for key_id in self.session_key_ids()? {
            let encoded = self
                .store
                .get(&key_id)?
                .ok_or(StorageError::KeyStoreUnavailable(None))?;
            let envelope: SessionKeyEnvelope = serde_json::from_slice(&encoded)?;
            let encrypted = EncryptedPayload {
                nonce: STANDARD
                    .decode(&envelope.nonce)
                    .map_err(|_| StorageError::AuthenticationFailed)?,
                ciphertext: STANDARD
                    .decode(&envelope.wrapped_key)
                    .map_err(|_| StorageError::AuthenticationFailed)?,
            };
            let plaintext = decrypt_with_root_candidates(
                &encrypted,
                target_root,
                fallback_root,
                &envelope_aad(envelope.session_id, &envelope.key_id, &self.root_key_id),
            )?;
            let rewrapped = EncryptedPayload::encrypt(
                target_root,
                &plaintext,
                &envelope_aad(envelope.session_id, &envelope.key_id, &self.root_key_id),
            )?;
            let replacement = SessionKeyEnvelope {
                nonce: STANDARD.encode(rewrapped.nonce),
                wrapped_key: STANDARD.encode(rewrapped.ciphertext),
                ..envelope
            };
            let encoded = Zeroizing::new(serde_json::to_vec(&replacement)?);
            self.store.put(&key_id, &encoded)?;
        }
        Ok(())
    }

    fn load_or_create_root_keys(&self) -> Result<(SecretKey, Option<SecretKey>), StorageError> {
        if self.store.get(&self.root_key_id)?.is_some() {
            self.root_keys()
        } else {
            let key = SecretKey::generate();
            self.write_root_record(&key, None)?;
            Ok((key, None))
        }
    }

    fn root_keys(&self) -> Result<(SecretKey, Option<SecretKey>), StorageError> {
        let bytes = self
            .store
            .get(&self.root_key_id)?
            .ok_or(StorageError::KeyStoreUnavailable(None))?;
        if bytes.len() == KEY_BYTES {
            return Ok((SecretKey::from_slice(&bytes)?, None));
        }
        let record: RootKeyRecord =
            serde_json::from_slice(&bytes).map_err(|_| StorageError::KeyStoreUnavailable(None))?;
        if record.schema_version != 1 {
            return Err(StorageError::KeyStoreUnavailable(None));
        }
        let current = Zeroizing::new(
            STANDARD
                .decode(&record.current)
                .map_err(|_| StorageError::KeyStoreUnavailable(None))?,
        );
        let previous = record
            .previous
            .as_ref()
            .map(|encoded| {
                let bytes = Zeroizing::new(
                    STANDARD
                        .decode(encoded)
                        .map_err(|_| StorageError::KeyStoreUnavailable(None))?,
                );
                SecretKey::from_slice(&bytes)
            })
            .transpose()?;
        Ok((SecretKey::from_slice(&current)?, previous))
    }

    fn write_root_record(
        &self,
        current: &SecretKey,
        previous: Option<&SecretKey>,
    ) -> Result<(), StorageError> {
        let record = RootKeyRecord {
            schema_version: 1,
            current: STANDARD.encode(current.expose()),
            previous: previous.map(|key| STANDARD.encode(key.expose())),
        };
        let encoded = Zeroizing::new(serde_json::to_vec(&record)?);
        self.store.put(&self.root_key_id, &encoded)
    }
}

fn decrypt_with_root_candidates(
    encrypted: &EncryptedPayload,
    current: &SecretKey,
    previous: Option<&SecretKey>,
    associated_data: &AssociatedData<'_>,
) -> Result<Zeroizing<Vec<u8>>, StorageError> {
    match encrypted.decrypt(current, associated_data) {
        Ok(plaintext) => Ok(plaintext),
        Err(StorageError::AuthenticationFailed) => previous
            .ok_or(StorageError::AuthenticationFailed)
            .and_then(|key| encrypted.decrypt(key, associated_data)),
        Err(error) => Err(error),
    }
}

fn envelope_aad<'a>(session_id: Uuid, key_id: &'a str, root_key_id: &'a str) -> AssociatedData<'a> {
    AssociatedData {
        schema_version: 1,
        session_id,
        object_id: key_id,
        sequence: 0,
        kind: root_key_id,
    }
}

fn validate_envelope(
    envelope: &SessionKeyEnvelope,
    session_id: Uuid,
    key_id: &str,
    root_key_id: &str,
) -> Result<(), StorageError> {
    if envelope.schema_version != 1
        || envelope.session_id != session_id
        || envelope.key_id != key_id
        || envelope.root_key_id != root_key_id
        || envelope.algorithm != "xchacha20-poly1305"
    {
        return Err(StorageError::AuthenticationFailed);
    }
    let wrapped = STANDARD
        .decode(&envelope.wrapped_key)
        .map_err(|_| StorageError::AuthenticationFailed)?;
    if wrapped.len() != KEY_BYTES + 16 {
        return Err(StorageError::AuthenticationFailed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap},
        sync::{Arc, Mutex},
    };

    use super::{
        CatalogPutOps, KeyManager, KeyStore, MemoryKeyStore, cataloged_put, platform_catalog_id,
    };
    use crate::StorageError;
    use uuid::Uuid;
    use zeroize::Zeroizing;

    #[derive(Default)]
    struct FaultCatalogState {
        entries: HashMap<String, Zeroizing<Vec<u8>>>,
        catalogs: HashMap<String, BTreeSet<String>>,
        catalog_writes_until_failure: Option<usize>,
        delete_calls: usize,
    }

    #[derive(Clone, Default)]
    struct FaultCatalogStore {
        state: Arc<Mutex<FaultCatalogState>>,
    }

    impl FaultCatalogStore {
        fn fail_next_catalog_write(&self) {
            self.fail_catalog_write_after(0);
        }

        fn fail_catalog_write_after(&self, successful_writes: usize) {
            self.state
                .lock()
                .expect("fault catalog mutex")
                .catalog_writes_until_failure = Some(successful_writes);
        }

        fn delete_calls(&self) -> usize {
            self.state.lock().expect("fault catalog mutex").delete_calls
        }

        fn entry_count(&self) -> usize {
            self.state
                .lock()
                .expect("fault catalog mutex")
                .entries
                .len()
        }
    }

    struct FaultCatalogOperations<'a> {
        state: &'a mut FaultCatalogState,
    }

    impl CatalogPutOps for FaultCatalogOperations<'_> {
        fn get_secret(&mut self, id: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError> {
            Ok(self.state.entries.get(id).cloned())
        }

        fn set_secret(&mut self, id: &str, secret: &[u8]) -> Result<(), StorageError> {
            self.state
                .entries
                .insert(id.to_owned(), Zeroizing::new(secret.to_vec()));
            Ok(())
        }

        fn delete_secret(&mut self, id: &str) -> Result<(), StorageError> {
            self.state.delete_calls += 1;
            self.state.entries.remove(id);
            Ok(())
        }

        fn read_catalog(&mut self, catalog_id: &str) -> Result<BTreeSet<String>, StorageError> {
            Ok(self
                .state
                .catalogs
                .get(catalog_id)
                .cloned()
                .unwrap_or_default())
        }

        fn write_catalog(
            &mut self,
            catalog_id: &str,
            catalog: &BTreeSet<String>,
        ) -> Result<(), StorageError> {
            if let Some(writes) = self.state.catalog_writes_until_failure.as_mut() {
                if *writes == 0 {
                    self.state.catalog_writes_until_failure = None;
                    return Err(StorageError::KeyStoreUnavailable(None));
                }
                *writes -= 1;
            }
            self.state
                .catalogs
                .insert(catalog_id.to_owned(), catalog.clone());
            Ok(())
        }
    }

    impl KeyStore for FaultCatalogStore {
        fn put(&self, id: &str, secret: &[u8]) -> Result<(), StorageError> {
            let catalog_id = platform_catalog_id(id)?;
            let mut state = self.state.lock().expect("fault catalog mutex");
            cataloged_put(
                &mut FaultCatalogOperations { state: &mut state },
                id,
                secret,
                &catalog_id,
            )
        }

        fn get(&self, id: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError> {
            Ok(self
                .state
                .lock()
                .expect("fault catalog mutex")
                .entries
                .get(id)
                .cloned())
        }

        fn delete(&self, id: &str) -> Result<(), StorageError> {
            let catalog_id = platform_catalog_id(id)?;
            let mut state = self.state.lock().expect("fault catalog mutex");
            state.entries.remove(id);
            if let Some(catalog) = state.catalogs.get_mut(&catalog_id) {
                catalog.remove(id);
            }
            Ok(())
        }

        fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
            let catalog_id = platform_catalog_id(prefix)?;
            Ok(self
                .state
                .lock()
                .expect("fault catalog mutex")
                .catalogs
                .get(&catalog_id)
                .into_iter()
                .flatten()
                .filter(|id| id.starts_with(prefix))
                .cloned()
                .collect())
        }
    }

    #[test]
    fn creates_unwraps_rotates_and_destroys_keys() {
        let store = MemoryKeyStore::new();
        let storage_id = Uuid::now_v7();
        let manager = KeyManager::for_storage(store.clone(), storage_id, b"database");
        let session_id = Uuid::now_v7();
        manager.create_session_key(session_id).expect("create key");
        let before = manager.session_key(session_id).expect("unwrap");

        manager.rotate_root_key().expect("rotate root");
        drop(manager);
        let reloaded_manager = KeyManager::for_storage(store.clone(), storage_id, b"database");
        let after = reloaded_manager
            .session_key(session_id)
            .expect("unwrap after rotate");
        assert_eq!(before.expose(), after.expose());
        assert!(
            store
                .get(&reloaded_manager.root_key_id)
                .expect("get root")
                .is_some()
        );

        reloaded_manager
            .destroy_session_key(session_id)
            .expect("destroy key");
        assert!(
            store
                .get(&reloaded_manager.session_key_id(session_id))
                .expect("get key")
                .is_none()
        );
    }

    #[test]
    fn fails_closed_when_store_is_unavailable() {
        let store = MemoryKeyStore::new();
        store.set_available(false);
        let manager = KeyManager::for_storage(store, Uuid::now_v7(), b"database");
        assert!(manager.create_session_key(Uuid::now_v7()).is_err());
    }

    #[test]
    fn root_rotation_only_rewraps_its_database_namespace() {
        let store = MemoryKeyStore::new();
        let storage_a = Uuid::now_v7();
        let storage_b = Uuid::now_v7();
        let session_id = Uuid::now_v7();
        let manager_a = KeyManager::for_storage(store.clone(), storage_a, b"database-a");
        let manager_b = KeyManager::for_storage(store.clone(), storage_b, b"database-b");
        manager_a.create_session_key(session_id).expect("A key");
        manager_b.create_session_key(session_id).expect("B key");
        let before_b = manager_b.session_key(session_id).expect("unwrap B");

        manager_a.rotate_root_key().expect("rotate A root");
        drop((manager_a, manager_b));
        let reloaded_a = KeyManager::for_storage(store.clone(), storage_a, b"database-a");
        let reloaded_b = KeyManager::for_storage(store, storage_b, b"database-b");
        reloaded_a.session_key(session_id).expect("unwrap A");
        let after_b = reloaded_b.session_key(session_id).expect("unwrap B");
        assert_eq!(before_b.expose(), after_b.expose());
    }

    #[test]
    fn platform_catalogs_are_partitioned_by_database_namespace() {
        let storage_id = Uuid::now_v7();
        let session_id = Uuid::now_v7();
        let first = format!(
            "workbench/storage/{storage_id}/{}/session/{session_id}/v1",
            "a".repeat(64)
        );
        let second = format!(
            "workbench/storage/{storage_id}/{}/session/{session_id}/v1",
            "b".repeat(64)
        );

        assert_ne!(
            platform_catalog_id(&first).expect("first catalog"),
            platform_catalog_id(&second).expect("second catalog")
        );
        assert!(platform_catalog_id("contract/first").is_err());
        assert!(platform_catalog_id("workbench/storage/").is_err());
    }

    #[test]
    fn catalog_failure_rolls_back_create_without_deleting_an_existing_credential() {
        let store = FaultCatalogStore::default();
        store.fail_next_catalog_write();
        let manager = KeyManager::for_storage(store.clone(), Uuid::now_v7(), b"database");

        assert!(manager.create_session_key(Uuid::now_v7()).is_err());
        assert_eq!(store.entry_count(), 0);
        assert_eq!(store.delete_calls(), 1, "only the new root is removed");
    }

    #[test]
    fn catalog_failure_during_rotation_restores_existing_credentials() {
        for successful_writes in [0, 1] {
            let store = FaultCatalogStore::default();
            let storage_id = Uuid::now_v7();
            let session_id = Uuid::now_v7();
            let manager = KeyManager::for_storage(store.clone(), storage_id, b"database");
            manager.create_session_key(session_id).expect("create key");
            let before = manager.session_key(session_id).expect("unwrap");
            let deletes_before = store.delete_calls();
            store.fail_catalog_write_after(successful_writes);

            assert!(manager.rotate_root_key().is_err());
            assert_eq!(
                store.delete_calls(),
                deletes_before,
                "rollback must not delete a pre-existing root or envelope"
            );
            drop(manager);
            let reloaded = KeyManager::for_storage(store, storage_id, b"database");
            let after = reloaded
                .session_key(session_id)
                .expect("unwrap after failed rotation");
            assert_eq!(before.expose(), after.expose());
        }
    }
}
