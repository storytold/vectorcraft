//! The key that marks a plug-in store's `grants.json` as written by VectorCraft. The file carries an
//! HMAC-SHA-256 ([RFC 2104](https://www.rfc-editor.org/rfc/rfc2104)) of the grants it holds and of
//! the plug-in folder it is in, made with a random 32-byte key that the user's key store keeps
//! ([`GrantKeyStore`]), never a file. A grants file without the MAC of that key for its folder (one
//! an older version wrote, one another program wrote or changed, one copied from another folder or
//! machine) grants nothing.
//!
//! The key store also keeps a generation for each plug-in folder, raised on every signed write of
//! its grants file and never lowered, and the file carries its generation inside the MAC. A file
//! whose generation is not the one kept (an earlier copy put back over the current file, or a write
//! whose generation the key store didn't keep or that failed after the raise) grants nothing either.
//!
//! The process uses one key store ([`set_grant_key_store`]); the apps use the system's
//! ([`use_os_grant_keys`]). In a process that sets none, every grants file counts as one whose key
//! store can't be reached: a grant holds for the session only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

/// The length of a key, in bytes, and of the MAC.
pub const KEY_LEN: usize = 32;

/// A key for the MAC of grants files. Its bytes are never shown: `{:?}` prints `GrantKey(..)`.
#[derive(Clone, PartialEq, Eq)]
pub struct GrantKey([u8; KEY_LEN]);

impl GrantKey {
    /// The key with the bytes `bytes`.
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        GrantKey(bytes)
    }

    /// A new key from the system's random source.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn random() -> Result<Self, KeyStoreError> {
        let mut bytes = [0u8; KEY_LEN];
        getrandom::fill(&mut bytes).map_err(|e| KeyStoreError(format!("no random numbers ({e})")))?;
        Ok(GrantKey(bytes))
    }

    /// The browser keeps no grants file.
    #[cfg(target_arch = "wasm32")]
    pub fn random() -> Result<Self, KeyStoreError> {
        Err(KeyStoreError("there is no key store".into()))
    }
}

impl std::fmt::Debug for GrantKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GrantKey(..)")
    }
}

/// Why a key store gave no key or didn't keep one. The message never holds key bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct KeyStoreError(pub String);

/// Where the key of grants files is kept.
pub trait GrantKeyStore: Send + Sync {
    /// The key kept, `None` when there is none; an error when the store can't be reached or read.
    fn key(&self) -> Result<Option<GrantKey>, KeyStoreError>;
    /// Keep `key` in place of any kept before.
    fn set_key(&self, key: &GrantKey) -> Result<(), KeyStoreError>;
    /// The key kept, after making and keeping a new one when there is none. The key returned is the
    /// one read back from the store, so a key another process kept first wins.
    fn create_key(&self) -> Result<GrantKey, KeyStoreError> {
        if let Some(key) = self.key()? {
            return Ok(key);
        }
        self.set_key(&GrantKey::random()?)?;
        self.key()?.ok_or_else(|| KeyStoreError("the key store didn't keep the key".into()))
    }
    /// The generation kept for the grants file of the plug-in store whose folder is `folder`
    /// ([`super::plugin_store_key`]), read from the store each time; `None` when none is kept.
    fn generation(&self, folder: &Path) -> Result<Option<u64>, KeyStoreError>;
    /// Keep `generation` for the grants file of the plug-in store whose folder is `folder`.
    fn set_generation(&self, folder: &Path, generation: u64) -> Result<(), KeyStoreError>;
}

/// A key store in memory: the key and the generations last as long as the store. For tests and
/// embedders.
#[derive(Default)]
pub struct MemoryGrantKeys {
    key: Mutex<Option<GrantKey>>,
    generations: Mutex<BTreeMap<PathBuf, u64>>,
}

impl MemoryGrantKeys {
    /// A store that holds `key`.
    pub fn with_key(key: GrantKey) -> Self {
        MemoryGrantKeys { key: Mutex::new(Some(key)), ..MemoryGrantKeys::default() }
    }
}

impl GrantKeyStore for MemoryGrantKeys {
    fn key(&self) -> Result<Option<GrantKey>, KeyStoreError> {
        Ok(self.key.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    fn set_key(&self, key: &GrantKey) -> Result<(), KeyStoreError> {
        *self.key.lock().unwrap_or_else(|e| e.into_inner()) = Some(key.clone());
        Ok(())
    }

    fn generation(&self, folder: &Path) -> Result<Option<u64>, KeyStoreError> {
        Ok(self.generations.lock().unwrap_or_else(|e| e.into_inner()).get(folder).copied())
    }

    fn set_generation(&self, folder: &Path, generation: u64) -> Result<(), KeyStoreError> {
        self.generations.lock().unwrap_or_else(|e| e.into_inner()).insert(folder.to_path_buf(), generation);
        Ok(())
    }
}

/// The key store of the process ([`set_grant_key_store`]).
static PROCESS_STORE: Mutex<Option<Arc<dyn GrantKeyStore>>> = Mutex::new(None);

/// The one memory store of [`keep_grant_keys_in_memory`].
static SHARED_MEMORY: LazyLock<Arc<MemoryGrantKeys>> = LazyLock::new(Arc::default);

#[cfg(test)]
thread_local! {
    /// Tests: the key store this thread uses in place of the process's.
    pub(crate) static THREAD_STORE: std::cell::RefCell<Option<Arc<dyn GrantKeyStore>>> = const { std::cell::RefCell::new(None) };
}

/// Use `store` for the grants files of every plug-in store in the process from now on (`None`: no
/// key store, so grants hold for the session only).
pub fn set_grant_key_store(store: Option<Arc<dyn GrantKeyStore>>) {
    *PROCESS_STORE.lock().unwrap_or_else(|e| e.into_inner()) = store;
}

/// Keep the key in memory for the rest of the process, one key for every caller: grants files
/// written in the process count in it, and not in another process. For tests.
pub fn keep_grant_keys_in_memory() {
    let shared: Arc<dyn GrantKeyStore> = SHARED_MEMORY.clone();
    set_grant_key_store(Some(shared));
}

/// Keep the key in the system's key store: Keychain on macOS, Credential Manager on Windows, the
/// Secret Service on Linux and the BSDs, as the generic secret `hmac-sha256-v1` of the service
/// `org.storytold.vectorcraft.plugin-grants`.
#[cfg(not(target_arch = "wasm32"))]
pub fn use_os_grant_keys() {
    set_grant_key_store(Some(Arc::new(OsGrantKeys::default())));
}

/// The key store in use: the thread's in tests, else the process's; in tests, the shared memory
/// store when the process set none.
fn current_store() -> Option<Arc<dyn GrantKeyStore>> {
    #[cfg(test)]
    if let Some(store) = THREAD_STORE.with_borrow(Clone::clone) {
        return Some(store);
    }
    let store = PROCESS_STORE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    #[cfg(test)]
    let store = store.or_else(|| Some(SHARED_MEMORY.clone() as Arc<dyn GrantKeyStore>));
    store
}

/// The key kept for grants files: `None` when there is none yet, an error when there is no key
/// store or it can't be reached.
pub(crate) fn current_key() -> Result<Option<GrantKey>, KeyStoreError> {
    current_store().ok_or_else(|| KeyStoreError("there is no key store".into()))?.key()
}

/// The key to sign a grants file with: the one kept, else a new one, kept first
/// ([`GrantKeyStore::create_key`]).
pub(crate) fn signing_key() -> Result<GrantKey, KeyStoreError> {
    let store = current_store().ok_or_else(|| KeyStoreError("there is no key store".into()))?;
    match store.key()? {
        Some(key) => Ok(key),
        None => store.create_key(),
    }
}

/// The generation kept for the grants file of the plug-in store whose folder is `folder`: 0 when
/// none is kept yet, an error when there is no key store or it can't be reached.
pub(crate) fn current_generation(folder: &Path) -> Result<u64, KeyStoreError> {
    Ok(current_store().ok_or_else(|| KeyStoreError("there is no key store".into()))?.generation(folder)?.unwrap_or(0))
}

/// Keep `generation` for the grants file of the plug-in store whose folder is `folder`. The kept
/// generation only goes up: one lower than the kept one is an error, and the kept one stays, since a
/// lower one could match a file signed before.
pub(crate) fn set_current_generation(folder: &Path, generation: u64) -> Result<(), KeyStoreError> {
    let store = current_store().ok_or_else(|| KeyStoreError("there is no key store".into()))?;
    if store.generation(folder)?.is_some_and(|kept| generation < kept) {
        return Err(KeyStoreError("the key store keeps a later generation".into()));
    }
    store.set_generation(folder, generation)
}

/// The start of every MAC input: what it is a MAC of, and the version of the file.
#[cfg(not(target_arch = "wasm32"))]
const DOMAIN: &[u8] = b"VectorCraft plug-in grants v1\0";

/// The MAC of `grants` (the grants list as compact JSON) at `generation` in the plug-in store whose
/// folder is `folder` ([`super::plugin_store_key`]), made with `key`. The folder and the generation
/// go in with their lengths first.
#[cfg(not(target_arch = "wasm32"))]
fn mac_of(key: &GrantKey, folder: &Path, generation: u64, grants: &[u8]) -> Option<hmac::Hmac<sha2::Sha256>> {
    use hmac::Mac;
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&key.0).ok()?;
    let folder = folder.as_os_str().as_encoded_bytes();
    let generation = generation.to_be_bytes();
    mac.update(DOMAIN);
    mac.update(&(folder.len() as u64).to_be_bytes());
    mac.update(folder);
    mac.update(&(generation.len() as u64).to_be_bytes());
    mac.update(&generation);
    mac.update(grants);
    Some(mac)
}

/// The MAC of `grants` at `generation` in `folder` with `key`, as 64 lower-case hexadecimal digits.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sign(key: &GrantKey, folder: &Path, generation: u64, grants: &[u8]) -> Option<String> {
    use hmac::Mac;
    Some(mac_of(key, folder, generation, grants)?.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether `tag` is the whole MAC of `grants` at `generation` in `folder` with `key` ([`sign`]),
/// compared in constant time; a shorter or longer tag is not.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn verify(key: &GrantKey, folder: &Path, generation: u64, grants: &[u8], tag: &str) -> bool {
    use hmac::Mac;
    let Some(tag) = from_hex(tag) else { return false };
    mac_of(key, folder, generation, grants).is_some_and(|mac| mac.verify_slice(&tag).is_ok())
}

/// The browser keeps no grants file.
#[cfg(target_arch = "wasm32")]
pub(crate) fn sign(_: &GrantKey, _: &Path, _: u64, _: &[u8]) -> Option<String> {
    None
}

/// The browser keeps no grants file.
#[cfg(target_arch = "wasm32")]
pub(crate) fn verify(_: &GrantKey, _: &Path, _: u64, _: &[u8], _: &str) -> bool {
    false
}

/// The bytes written as lower-case hexadecimal digits in `tag`, at most twice [`KEY_LEN`] of them;
/// `None` for anything else.
#[cfg(not(target_arch = "wasm32"))]
fn from_hex(tag: &str) -> Option<Vec<u8>> {
    let digits = tag.as_bytes();
    let (pairs, rest) = digits.as_chunks::<2>();
    if !rest.is_empty() || pairs.len() > KEY_LEN {
        return None;
    }
    let digit = |d: u8| match d {
        b'0'..=b'9' => Some(d - b'0'),
        b'a'..=b'f' => Some(d - b'a' + 10),
        _ => None,
    };
    pairs.iter().map(|&[high, low]| Some((digit(high)? << 4) | digit(low)?)).collect()
}

/// The system's key store, through `keyring-core` and the store crate of the platform. The key is
/// read from the store each time it is needed. The key and the generations are kept as lower-case
/// hexadecimal text, since some stores (KDE Wallet behind the Secret Service) keep only UTF-8.
#[cfg(not(target_arch = "wasm32"))]
struct OsGrantKeys {
    /// Held while a key is kept, so that the threads of the process that make the first key at the
    /// same time keep one.
    making: Mutex<()>,
    /// The connection to the system's key store, once one is made. A failed connection is not
    /// kept, and a kept one is dropped when the key store can't be reached through it or fails
    /// ([`OsGrantKeys::failed`]): the next call connects again, so a key store that starts after
    /// VectorCraft, or restarts, is used.
    store: Mutex<Option<Arc<keyring_core::CredentialStore>>>,
    /// How a connection is made ([`connect`]).
    connect: fn() -> Result<Arc<keyring_core::CredentialStore>, keyring_core::Error>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for OsGrantKeys {
    fn default() -> Self {
        OsGrantKeys { making: Mutex::default(), store: Mutex::default(), connect }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl OsGrantKeys {
    const SERVICE: &str = "org.storytold.vectorcraft.plugin-grants";
    const ACCOUNT: &str = "hmac-sha256-v1";

    /// The system's key store: the connection made before, else a new one.
    fn store(&self) -> Result<Arc<keyring_core::CredentialStore>, KeyStoreError> {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = store.as_ref() {
            return Ok(store.clone());
        }
        Ok(store.insert((self.connect)().map_err(store_error)?).clone())
    }

    /// What the key store error `e` says ([`store_error`]). When the key store can't be reached or
    /// fails, the connection is dropped first, so the next call makes a new one: the Secret Service
    /// loses the session of a connection when it restarts. An entry that isn't there keeps it.
    fn failed(&self, e: keyring_core::Error) -> KeyStoreError {
        if matches!(e, keyring_core::Error::NoStorageAccess(_) | keyring_core::Error::PlatformFailure(_)) {
            *self.store.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        store_error(e)
    }

    fn entry(&self, account: &str) -> Result<keyring_core::Entry, KeyStoreError> {
        self.store()?.build(Self::SERVICE, account, None).map_err(|e| self.failed(e))
    }

    fn key_entry(&self) -> Result<keyring_core::Entry, KeyStoreError> {
        self.entry(Self::ACCOUNT)
    }

    /// The key the system's key store keeps, read from it.
    fn stored(&self, entry: &keyring_core::Entry) -> Result<Option<GrantKey>, KeyStoreError> {
        match entry.get_secret() {
            // A secret that is not 64 hexadecimal digits is not a key this version made: there is none.
            Ok(secret) => Ok(std::str::from_utf8(&secret)
                .ok()
                .and_then(from_hex)
                .and_then(|bytes| <[u8; KEY_LEN]>::try_from(bytes.as_slice()).ok())
                .map(GrantKey)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(self.failed(e)),
        }
    }

    /// The entry of the generation of the plug-in store whose folder is `folder`: the account
    /// `generation-v1-` followed by the SHA-256 of the folder in hexadecimal digits.
    fn generation_entry(&self, folder: &Path) -> Result<keyring_core::Entry, KeyStoreError> {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(folder.as_os_str().as_encoded_bytes());
        self.entry(&format!("generation-v1-{}", to_hex(&digest)))
    }
}

/// A connection to the system's key store: Keychain on macOS, Credential Manager on Windows, the
/// Secret Service on Linux and the BSDs. The same stores, made the same way, that `keyring` 4.2
/// uses, so entries it kept are read.
#[cfg(not(target_arch = "wasm32"))]
fn connect() -> Result<Arc<keyring_core::CredentialStore>, keyring_core::Error> {
    #[cfg(target_os = "macos")]
    {
        let store: Arc<keyring_core::CredentialStore> = apple_native_keyring_store::keychain::Store::new()?;
        Ok(store)
    }
    #[cfg(windows)]
    {
        let store: Arc<keyring_core::CredentialStore> = windows_native_keyring_store::Store::new()?;
        Ok(store)
    }
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    {
        let store: Arc<keyring_core::CredentialStore> = zbus_secret_service_keyring_store::Store::new()?;
        Ok(store)
    }
    #[cfg(not(any(target_os = "macos", windows, all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))))]
    Err(keyring_core::Error::NoDefaultStore)
}

/// `bytes` as lower-case hexadecimal digits.
#[cfg(not(target_arch = "wasm32"))]
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(not(target_arch = "wasm32"))]
impl GrantKeyStore for OsGrantKeys {
    /// Read from the key store each time, so that a key removed or replaced there is not used.
    fn key(&self) -> Result<Option<GrantKey>, KeyStoreError> {
        self.stored(&self.key_entry()?)
    }

    fn set_key(&self, key: &GrantKey) -> Result<(), KeyStoreError> {
        let _making = self.making.lock().unwrap_or_else(|e| e.into_inner());
        self.key_entry()?.set_secret(to_hex(&key.0).as_bytes()).map_err(|e| self.failed(e))
    }

    /// Reads the key store again and makes the key only while holding [`creation_lock`], so that
    /// processes of this user that make the first key at the same time all keep the first one made.
    fn create_key(&self) -> Result<GrantKey, KeyStoreError> {
        let _making = self.making.lock().unwrap_or_else(|e| e.into_inner());
        let entry = self.key_entry()?;
        if let Some(key) = self.stored(&entry)? {
            return Ok(key);
        }
        let _turn = creation_lock()?;
        match self.stored(&entry)? {
            Some(key) => Ok(key),
            None => {
                entry.set_secret(to_hex(&GrantKey::random()?.0).as_bytes()).map_err(|e| self.failed(e))?;
                self.stored(&entry)?.ok_or_else(|| KeyStoreError("the key store didn't keep the key".into()))
            }
        }
    }

    /// Read from the key store each time, since another process on the folder may have raised it.
    /// A kept value that is not 16 hexadecimal digits is an error, so that nothing counts as signed
    /// with it.
    fn generation(&self, folder: &Path) -> Result<Option<u64>, KeyStoreError> {
        match self.generation_entry(folder)?.get_secret() {
            Ok(secret) => std::str::from_utf8(&secret)
                .ok()
                .and_then(from_hex)
                .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                .map(|bytes| Some(u64::from_be_bytes(bytes)))
                .ok_or_else(|| KeyStoreError("the key store holds a generation that is not 16 hexadecimal digits".into())),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(self.failed(e)),
        }
    }

    fn set_generation(&self, folder: &Path, generation: u64) -> Result<(), KeyStoreError> {
        self.generation_entry(folder)?.set_secret(to_hex(&generation.to_be_bytes()).as_bytes()).map_err(|e| self.failed(e))
    }
}

/// `vectorcraft-plugin-grant-key.lock` in a folder of this user's own, locked until the file is
/// dropped, waiting while another process holds it: `%LOCALAPPDATA%` on Windows, the temporary
/// folder on macOS, `$XDG_RUNTIME_DIR` elsewhere. An error when there is no such folder or the file
/// can't be locked: no key is made then.
#[cfg(not(target_arch = "wasm32"))]
fn creation_lock() -> Result<std::fs::File, KeyStoreError> {
    #[cfg(windows)]
    let (dir, variable) = (std::env::var_os("LOCALAPPDATA"), "%LOCALAPPDATA%");
    #[cfg(target_os = "macos")]
    let (dir, variable) = (Some(std::env::temp_dir().into_os_string()), "$TMPDIR");
    #[cfg(not(any(windows, target_os = "macos")))]
    let (dir, variable) = (std::env::var_os("XDG_RUNTIME_DIR"), "$XDG_RUNTIME_DIR");
    let dir = dir.map(std::path::PathBuf::from).filter(|dir| dir.is_absolute());
    let dir = dir.ok_or_else(|| KeyStoreError(format!("there is no folder for the key's lock ({variable} is not set)")))?;
    let file = std::fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("vectorcraft-plugin-grant-key.lock"))
        .map_err(|e| KeyStoreError(format!("can't open the key's lock ({e})")))?;
    file.lock().map_err(|e| KeyStoreError(format!("can't lock the key's lock ({e})")))?;
    Ok(file)
}

/// What a key store error says, without the secret some of them hold.
#[cfg(not(target_arch = "wasm32"))]
fn store_error(e: keyring_core::Error) -> KeyStoreError {
    KeyStoreError(match e {
        keyring_core::Error::NoStorageAccess(e) => format!("the key store is locked or can't be reached ({e})"),
        keyring_core::Error::PlatformFailure(e) => format!("the key store failed ({e})"),
        keyring_core::Error::NoDefaultStore => "there is no key store".into(),
        _ => "the key store can't read or keep the key".into(),
    })
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use keyring_core::api::CredentialStoreApi;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Connections asked for from [`flaky`].
    static TRIES: AtomicUsize = AtomicUsize::new(0);
    /// The store [`flaky`] connects to.
    static MOCK: LazyLock<Arc<keyring_core::mock::Store>> = LazyLock::new(|| keyring_core::mock::Store::new().unwrap());

    /// A key store that can't be reached on the first try and is reached on every later one.
    fn flaky() -> Result<Arc<keyring_core::CredentialStore>, keyring_core::Error> {
        if TRIES.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(keyring_core::Error::NoDefaultStore);
        }
        let store: Arc<keyring_core::CredentialStore> = MOCK.clone();
        Ok(store)
    }

    /// Connections asked for from [`restarting`].
    static RESTARTS: AtomicUsize = AtomicUsize::new(0);
    /// The store [`restarting`] connects to first.
    static BEFORE: LazyLock<Arc<keyring_core::mock::Store>> = LazyLock::new(|| keyring_core::mock::Store::new().unwrap());
    /// The store [`restarting`] connects to on every later try.
    static AFTER: LazyLock<Arc<keyring_core::mock::Store>> = LazyLock::new(|| keyring_core::mock::Store::new().unwrap());

    /// A key store that restarts after the first connection: later connections reach another store.
    fn restarting() -> Result<Arc<keyring_core::CredentialStore>, keyring_core::Error> {
        let store: Arc<keyring_core::CredentialStore> = if RESTARTS.fetch_add(1, Ordering::SeqCst) == 0 { BEFORE.clone() } else { AFTER.clone() };
        Ok(store)
    }

    /// A connection that the key store can't be reached through is dropped, and the next call
    /// connects again; an entry that isn't there keeps the connection.
    #[test]
    fn the_system_key_store_reconnects_after_it_restarts() {
        let keys = OsGrantKeys { connect: restarting, ..OsGrantKeys::default() };
        assert_eq!(keys.key().unwrap(), None, "no key is kept yet");
        keys.set_key(&GrantKey::from_bytes([0xc1; KEY_LEN])).unwrap();
        assert_eq!(RESTARTS.load(Ordering::SeqCst), 1, "an entry that isn't there keeps the connection");

        AFTER.build(OsGrantKeys::SERVICE, OsGrantKeys::ACCOUNT, None).unwrap().set_secret("d2".repeat(KEY_LEN).as_bytes()).unwrap();
        let entry = BEFORE.build(OsGrantKeys::SERVICE, OsGrantKeys::ACCOUNT, None).unwrap();
        let cred: &keyring_core::mock::Cred = entry.as_any().downcast_ref().unwrap();
        cred.set_error(keyring_core::Error::NoStorageAccess("the session is gone".into()));
        assert!(keys.key().is_err(), "the connection fails");
        assert_eq!(keys.key().unwrap(), Some(GrantKey::from_bytes([0xd2; KEY_LEN])), "the next call reads the key through a new connection");
        assert_eq!(RESTARTS.load(Ordering::SeqCst), 2);
    }

    /// The secret `MOCK` keeps for `account`, as it is kept.
    fn kept(account: &str) -> Vec<u8> {
        MOCK.build(OsGrantKeys::SERVICE, account, None).unwrap().get_secret().unwrap()
    }

    /// The system key store is connected again after a connection fails, the key and the
    /// generations are kept as hexadecimal text, and the key is read from the store each time; a
    /// value kept as raw bytes is not read as a key or a generation.
    #[test]
    fn the_system_key_store_reconnects_and_keeps_text() {
        let keys = OsGrantKeys { connect: flaky, ..OsGrantKeys::default() };
        assert!(keys.key().is_err(), "the first connection fails");
        let key = GrantKey::from_bytes([0xa7; KEY_LEN]);
        keys.set_key(&key).unwrap();
        assert_eq!(kept(OsGrantKeys::ACCOUNT), "a7".repeat(KEY_LEN).into_bytes(), "the key is kept as text");
        assert_eq!(OsGrantKeys { connect: flaky, ..OsGrantKeys::default() }.key().unwrap(), Some(key), "and read back");

        let folder = Path::new("/plug-ins");
        keys.set_generation(folder, 0x1_0000_0005).unwrap();
        let account = format!("generation-v1-{}", to_hex(&<sha2::Sha256 as sha2::Digest>::digest(folder.as_os_str().as_encoded_bytes())));
        assert_eq!(kept(&account), b"0000000100000005", "the generation is kept as text");
        assert_eq!(keys.generation(folder).unwrap(), Some(0x1_0000_0005));

        MOCK.build(OsGrantKeys::SERVICE, &account, None).unwrap().set_secret(&5u64.to_be_bytes()).unwrap();
        assert!(keys.generation(folder).is_err(), "a generation kept as raw bytes is an error");
        MOCK.build(OsGrantKeys::SERVICE, OsGrantKeys::ACCOUNT, None).unwrap().set_secret(&[0xa7; KEY_LEN]).unwrap();
        assert_eq!(keys.key().unwrap(), None, "a key kept as raw bytes is none, also after a key was read");
        MOCK.build(OsGrantKeys::SERVICE, OsGrantKeys::ACCOUNT, None).unwrap().set_secret("b8".repeat(KEY_LEN).as_bytes()).unwrap();
        assert_eq!(keys.key().unwrap(), Some(GrantKey::from_bytes([0xb8; KEY_LEN])), "a key replaced in the key store is the one used");
    }
}
