//! Cross-backend conformance: every scenario MUST behave identically
//! across MemStore, JsonStore (file-backed), and SqliteStore.
//!
//! Each backend ships its own unit tests in its `mod.rs`; this layer
//! catches *divergence* — the bug class where one backend silently
//! diverges from another's atomic / dedupe / count semantics.

use std::collections::HashMap;

use webylib_storage::{JsonStore, MemStore, SqliteStore, Store, StoreError};

fn each_backend(test: impl Fn(&dyn Store, &str)) {
    let mem = MemStore::new();
    test(&mem, "mem");

    let tmp = tempfile::NamedTempFile::new().expect("tmpfile");
    let json = JsonStore::new(Some(tmp.path().to_path_buf()));
    test(&json, "json");

    let sqlite = SqliteStore::open_in_memory().expect("sqlite");
    test(&sqlite, "sqlite");
}

#[test]
fn meta_roundtrip_is_uniform() {
    each_backend(|s, name| {
        s.set_meta("foo", "bar").unwrap_or_else(|e| panic!("[{name}] set: {e}"));
        assert_eq!(
            s.get_meta("foo").unwrap_or_else(|e| panic!("[{name}] get: {e}")),
            Some("bar".into()),
            "[{name}] roundtrip"
        );
        s.set_meta("foo", "baz").unwrap();
        assert_eq!(s.get_meta("foo").unwrap(), Some("baz".into()), "[{name}] overwrite");
        assert_eq!(s.get_meta("missing").unwrap(), None, "[{name}] missing → None");
    });
}

#[test]
fn get_all_meta_returns_every_key() {
    each_backend(|s, name| {
        s.set_meta("a", "1").unwrap();
        s.set_meta("b", "2").unwrap();
        s.set_meta("c", "3").unwrap();
        let all: HashMap<_, _> = s.get_all_meta().unwrap();
        assert_eq!(all.len(), 3, "[{name}] count");
        assert_eq!(all.get("a"), Some(&"1".to_string()), "[{name}] a");
        assert_eq!(all.get("b"), Some(&"2".to_string()), "[{name}] b");
        assert_eq!(all.get("c"), Some(&"3".to_string()), "[{name}] c");
    });
}

#[test]
fn output_insert_then_unspent_roundtrip() {
    each_backend(|s, name| {
        s.insert_output(&[1, 2, 3], "alpha", 100).unwrap();
        s.insert_output(&[4, 5, 6], "beta", 200).unwrap();
        let unspent = s.get_unspent().unwrap();
        assert_eq!(unspent.len(), 2, "[{name}] count");
        assert_eq!(s.sum_unspent().unwrap(), 300, "[{name}] sum");
    });
}

#[test]
fn duplicate_secret_hash_returns_constraint_on_every_backend() {
    each_backend(|s, name| {
        s.insert_output(&[7], "x", 10).unwrap();
        let err = s.insert_output(&[7], "y", 20).expect_err(name);
        assert!(matches!(err, StoreError::Constraint(_)), "[{name}] not Constraint: {err:?}");
    });
}

#[test]
fn mark_spent_moves_unspent_to_spent() {
    each_backend(|s, name| {
        s.insert_output(&[10], "alpha", 100).unwrap();
        assert_eq!(s.count_unspent().unwrap(), 1, "[{name}] before");
        s.mark_spent(&[10]).unwrap();
        assert_eq!(s.count_unspent().unwrap(), 0, "[{name}] after");
        assert_eq!(s.sum_unspent().unwrap(), 0, "[{name}] sum");
    });
}

#[test]
fn spent_hashes_dedupe() {
    each_backend(|s, name| {
        s.insert_spent_hash(&[1, 2, 3]).unwrap();
        s.insert_spent_hash(&[1, 2, 3]).unwrap(); // idempotent
        s.insert_spent_hash(&[4, 5, 6]).unwrap();
        assert_eq!(s.count_spent_hashes().unwrap(), 2, "[{name}] dedupe");
    });
}

#[test]
fn depth_get_set_uniform() {
    each_backend(|s, name| {
        assert_eq!(s.get_depth("Receive").unwrap(), 0, "[{name}] missing → 0");
        s.set_depth("Receive", 5).unwrap();
        assert_eq!(s.get_depth("Receive").unwrap(), 5, "[{name}] set");
        s.set_depth("Receive", 12).unwrap();
        assert_eq!(s.get_depth("Receive").unwrap(), 12, "[{name}] overwrite");
        s.set_depth("Pay", 3).unwrap();
        let all = s.get_all_depths().unwrap();
        assert_eq!(all.len(), 2, "[{name}] all len");
    });
}

#[test]
fn atomic_commits_on_ok_uniformly() {
    each_backend(|s, name| {
        s.atomic(&mut |inner| {
            inner.insert_output(&[1], "a", 100)?;
            inner.insert_output(&[2], "b", 200)?;
            Ok(())
        })
        .unwrap_or_else(|e| panic!("[{name}] atomic: {e}"));
        assert_eq!(s.count_unspent().unwrap(), 2, "[{name}] count");
        assert_eq!(s.sum_unspent().unwrap(), 300, "[{name}] sum");
    });
}

#[test]
fn atomic_rolls_back_on_err_uniformly() {
    each_backend(|s, name| {
        s.insert_output(&[99], "preexisting", 50).unwrap();
        let r: Result<(), StoreError> = s.atomic(&mut |inner| {
            inner.insert_output(&[1], "a", 100)?;
            inner.insert_output(&[2], "b", 200)?;
            Err(StoreError::Backend("simulated failure".into()))
        });
        assert!(r.is_err(), "[{name}] err propagates");
        assert_eq!(
            s.count_unspent().unwrap(),
            1,
            "[{name}] partial commit visible — rollback broken"
        );
        assert_eq!(s.sum_unspent().unwrap(), 50, "[{name}] sum after rollback");
    });
}

#[test]
fn clear_all_resets_everything() {
    each_backend(|s, name| {
        s.set_meta("k", "v").unwrap();
        s.insert_output(&[1], "a", 100).unwrap();
        s.insert_spent_hash(&[2]).unwrap();
        s.set_depth("Receive", 7).unwrap();

        s.clear_all().unwrap();

        assert!(s.get_all_meta().unwrap().is_empty(), "[{name}] meta");
        assert_eq!(s.count_outputs().unwrap(), 0, "[{name}] outputs");
        assert_eq!(s.count_spent_hashes().unwrap(), 0, "[{name}] spent");
        // depths can either be absent or zero on clear; both are fine.
        assert_eq!(s.get_depth("Receive").unwrap(), 0, "[{name}] depth");
    });
}

#[test]
fn update_output_amount_changes_sum() {
    each_backend(|s, name| {
        s.insert_output(&[1], "a", 100).unwrap();
        assert_eq!(s.sum_unspent().unwrap(), 100, "[{name}] before");
        s.update_output_amount(&[1], 250).unwrap();
        assert_eq!(s.sum_unspent().unwrap(), 250, "[{name}] after");
    });
}
