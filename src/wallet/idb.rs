//! IndexedDB persistence for wallet state (WASM only).
//!
//! Async `load` / `save` / `delete` for JSON-serialized MemState.
//! The `Store` trait stays synchronous (in-memory). IndexedDB is the
//! persistence layer — called explicitly by Wallet::save_to_idb / open_from_idb.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{IdbDatabase, IdbTransactionMode};

use crate::error::{Error, Result};

const DB_VERSION: u32 = 1;
const STORE_NAME: &str = "wallet_state";

fn db_name(network: &str) -> String {
    format!("webylib-wallet-{}", network)
}

/// Await an IdbRequest via onsuccess/onerror callbacks.
async fn await_request(req: &web_sys::IdbRequest) -> std::result::Result<(), JsValue> {
    let (tx, rx) = futures_channel::oneshot::channel::<std::result::Result<(), JsValue>>();
    let tx = std::rc::Rc::new(std::cell::RefCell::new(Some(tx)));

    let tx2 = tx.clone();
    let on_success = Closure::once(move |_: web_sys::Event| {
        if let Some(tx) = tx2.borrow_mut().take() {
            let _ = tx.send(Ok(()));
        }
    });
    let on_error = Closure::once(move |e: web_sys::Event| {
        if let Some(tx) = tx.borrow_mut().take() {
            let _ = tx.send(Err(e.into()));
        }
    });
    req.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
    req.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_success.forget();
    on_error.forget();
    rx.await
        .unwrap_or(Err(JsValue::from_str("channel dropped")))
}

/// Open (or create) the IndexedDB database for a network.
async fn open_db(network: &str) -> Result<IdbDatabase> {
    let window = web_sys::window().ok_or_else(|| Error::wallet("no window"))?;
    let factory = window
        .indexed_db()
        .map_err(|_| Error::wallet("IndexedDB not available"))?
        .ok_or_else(|| Error::wallet("IndexedDB not available"))?;

    let open_req = factory
        .open_with_u32(&db_name(network), DB_VERSION)
        .map_err(|e| Error::wallet(format!("IDB open: {:?}", e)))?;

    let on_upgrade = Closure::once(move |event: web_sys::IdbVersionChangeEvent| {
        let req: web_sys::IdbOpenDbRequest = event.target().unwrap().dyn_into().unwrap();
        let db: IdbDatabase = req.result().unwrap().dyn_into().unwrap();
        if !db.object_store_names().contains(STORE_NAME) {
            db.create_object_store(STORE_NAME).unwrap();
        }
    });
    open_req.set_onupgradeneeded(Some(on_upgrade.as_ref().unchecked_ref()));
    on_upgrade.forget();

    await_request(open_req.as_ref())
        .await
        .map_err(|e| Error::wallet(format!("IDB open await: {:?}", e)))?;
    open_req
        .result()
        .map_err(|e| Error::wallet(format!("IDB open result: {:?}", e)))?
        .dyn_into::<IdbDatabase>()
        .map_err(|_| Error::wallet("IDB: result is not a database"))
}

/// Load wallet state JSON from IndexedDB.
pub async fn load(network: &str, key: &str) -> Result<Option<String>> {
    let db = open_db(network).await?;
    let tx = db
        .transaction_with_str_and_mode(STORE_NAME, IdbTransactionMode::Readonly)
        .map_err(|e| Error::wallet(format!("IDB tx: {:?}", e)))?;
    let store = tx
        .object_store(STORE_NAME)
        .map_err(|e| Error::wallet(format!("IDB store: {:?}", e)))?;
    let req = store
        .get(&JsValue::from_str(key))
        .map_err(|e| Error::wallet(format!("IDB get: {:?}", e)))?;

    await_request(&req)
        .await
        .map_err(|e| Error::wallet(format!("IDB get await: {:?}", e)))?;

    let result = req
        .result()
        .map_err(|e| Error::wallet(format!("IDB get result: {:?}", e)))?;
    if result.is_undefined() || result.is_null() {
        Ok(None)
    } else {
        result
            .as_string()
            .map(Some)
            .ok_or_else(|| Error::wallet("IDB: stored value is not a string"))
    }
}

/// Save wallet state JSON to IndexedDB.
pub async fn save(network: &str, key: &str, json: &str) -> Result<()> {
    let db = open_db(network).await?;
    let tx = db
        .transaction_with_str_and_mode(STORE_NAME, IdbTransactionMode::Readwrite)
        .map_err(|e| Error::wallet(format!("IDB tx: {:?}", e)))?;
    let store = tx
        .object_store(STORE_NAME)
        .map_err(|e| Error::wallet(format!("IDB store: {:?}", e)))?;
    let req = store
        .put_with_key(&JsValue::from_str(json), &JsValue::from_str(key))
        .map_err(|e| Error::wallet(format!("IDB put: {:?}", e)))?;

    await_request(&req)
        .await
        .map_err(|e| Error::wallet(format!("IDB put await: {:?}", e)))?;
    Ok(())
}

/// Delete a key from IndexedDB.
pub async fn delete(network: &str, key: &str) -> Result<()> {
    let db = open_db(network).await?;
    let tx = db
        .transaction_with_str_and_mode(STORE_NAME, IdbTransactionMode::Readwrite)
        .map_err(|e| Error::wallet(format!("IDB tx: {:?}", e)))?;
    let store = tx
        .object_store(STORE_NAME)
        .map_err(|e| Error::wallet(format!("IDB store: {:?}", e)))?;
    let req = store
        .delete(&JsValue::from_str(key))
        .map_err(|e| Error::wallet(format!("IDB delete: {:?}", e)))?;

    await_request(&req)
        .await
        .map_err(|e| Error::wallet(format!("IDB delete await: {:?}", e)))?;
    Ok(())
}

/// Delete the entire database for a network.
pub async fn delete_db(network: &str) -> Result<()> {
    let window = web_sys::window().ok_or_else(|| Error::wallet("no window"))?;
    let factory = window
        .indexed_db()
        .map_err(|_| Error::wallet("IndexedDB not available"))?
        .ok_or_else(|| Error::wallet("IndexedDB not available"))?;
    let req = factory
        .delete_database(&db_name(network))
        .map_err(|e| Error::wallet(format!("IDB delete_database: {:?}", e)))?;

    await_request(req.as_ref())
        .await
        .map_err(|e| Error::wallet(format!("IDB delete_database await: {:?}", e)))?;
    Ok(())
}
