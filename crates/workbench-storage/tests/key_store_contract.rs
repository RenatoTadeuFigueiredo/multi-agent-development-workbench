use uuid::Uuid;
use workbench_storage::{KeyStore, MemoryKeyStore, PlatformKeyStore};

fn exercise_key_store(store: &dyn KeyStore, prefix: &str) {
    let first = format!("{prefix}/first");
    let second = format!("{prefix}/second");
    store.put(&first, b"first-secret").expect("put first key");
    store
        .put(&second, b"second-secret")
        .expect("put second key");

    assert_eq!(
        store
            .get(&first)
            .expect("get first key")
            .expect("first key exists")
            .as_slice(),
        b"first-secret"
    );
    assert_eq!(
        store.list(prefix).expect("list keys"),
        vec![first.clone(), second.clone()]
    );

    store.delete(&first).expect("delete first key");
    store.delete(&second).expect("delete second key");
    assert!(store.get(&first).expect("read deleted key").is_none());
    assert!(store.list(prefix).expect("empty list").is_empty());
}

#[test]
fn memory_key_store_obeys_the_common_contract() {
    let store = MemoryKeyStore::new();
    exercise_key_store(&store, "contract/memory");
}

#[test]
#[ignore = "requires the unlocked platform credential store"]
fn platform_key_store_obeys_the_common_contract() {
    let store = PlatformKeyStore::new();
    let prefix = format!(
        "workbench/storage/{}/{}/contract",
        Uuid::now_v7(),
        Uuid::now_v7()
    );
    exercise_key_store(&store, &prefix);
}
