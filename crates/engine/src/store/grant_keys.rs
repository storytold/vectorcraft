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

/// The system's key store, through `keyring`. The key is read once and kept for the process.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct OsGrantKeys {
    read: Mutex<Option<GrantKey>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl OsGrantKeys {
    const SERVICE: &str = "org.storytold.vectorcraft.plugin-grants";
    const ACCOUNT: &str = "hmac-sha256-v1";

    fn entry() -> Result<keyring::Entry, KeyStoreError> {
        keyring::Entry::new(Self::SERVICE, Self::ACCOUNT).map_err(store_error)
    }

    /// The key the system's key store keeps, read from it.
    fn stored(entry: &keyring::Entry) -> Result<Option<GrantKey>, KeyStoreError> {
        match entry.get_secret() {
            // A secret of another length is not a key this version made: there is none.
            Ok(secret) => Ok(<[u8; KEY_LEN]>::try_from(secret.as_slice()).ok().map(GrantKey)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_error(e)),
        }
    }

    /// The entry of the generation of the plug-in store whose folder is `folder`: the account
    /// `generation-v1-` followed by the SHA-256 of the folder in hexadecimal digits.
    fn generation_entry(folder: &Path) -> Result<keyring::Entry, KeyStoreError> {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(folder.as_os_str().as_encoded_bytes());
        let account: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        keyring::Entry::new(Self::SERVICE, &format!("generation-v1-{account}")).map_err(store_error)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl GrantKeyStore for OsGrantKeys {
    fn key(&self) -> Result<Option<GrantKey>, KeyStoreError> {
        let mut read = self.read.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(key) = read.as_ref() {
            return Ok(Some(key.clone()));
        }
        Ok(Self::stored(&Self::entry()?)?.map(|key| read.insert(key).clone()))
    }

    fn set_key(&self, key: &GrantKey) -> Result<(), KeyStoreError> {
        let mut read = self.read.lock().unwrap_or_else(|e| e.into_inner());
        Self::entry()?.set_secret(&key.0).map_err(store_error)?;
        *read = Some(key.clone());
        Ok(())
    }

    /// Reads the key store again, not the key read before, and makes the key only while holding
    /// [`creation_lock`], so that processes of this user that make the first key at the same time
    /// all keep the first one made.
    fn create_key(&self) -> Result<GrantKey, KeyStoreError> {
        let mut read = self.read.lock().unwrap_or_else(|e| e.into_inner());
        let entry = Self::entry()?;
        if let Some(key) = Self::stored(&entry)? {
            return Ok(read.insert(key).clone());
        }
        let _turn = creation_lock()?;
        let key = match Self::stored(&entry)? {
            Some(key) => key,
            None => {
                entry.set_secret(&GrantKey::random()?.0).map_err(store_error)?;
                Self::stored(&entry)?.ok_or_else(|| KeyStoreError("the key store didn't keep the key".into()))?
            }
        };
        *read = Some(key.clone());
        Ok(key)
    }

    /// Read from the key store each time, since another process on the folder may have raised it.
    /// A kept value that is not 8 bytes is an error, so that nothing counts as signed with it.
    fn generation(&self, folder: &Path) -> Result<Option<u64>, KeyStoreError> {
        match Self::generation_entry(folder)?.get_secret() {
            Ok(secret) => <[u8; 8]>::try_from(secret.as_slice())
                .map(|bytes| Some(u64::from_be_bytes(bytes)))
                .map_err(|_| KeyStoreError("the key store holds a generation of another length".into())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_error(e)),
        }
    }

    fn set_generation(&self, folder: &Path, generation: u64) -> Result<(), KeyStoreError> {
        Self::generation_entry(folder)?.set_secret(&generation.to_be_bytes()).map_err(store_error)
    }
}

/// `vectorcraft-plugin-grant-key.lock` in a folder of this user's own, locked until the file is
/// dropped, waiting while another process holds it: the temporary folder on Windows and macOS,
/// `$XDG_RUNTIME_DIR` elsewhere. An error when there is no such folder or the file can't be
/// locked: no key is made then.
#[cfg(not(target_arch = "wasm32"))]
fn creation_lock() -> Result<std::fs::File, KeyStoreError> {
    #[cfg(any(windows, target_os = "macos"))]
    let dir = Some(std::env::temp_dir());
    #[cfg(not(any(windows, target_os = "macos")))]
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).filter(|dir| dir.is_absolute());
    let dir = dir.ok_or_else(|| KeyStoreError("there is no folder for the key's lock ($XDG_RUNTIME_DIR is not set)".into()))?;
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
fn store_error(e: keyring::Error) -> KeyStoreError {
    KeyStoreError(match e {
        keyring::Error::NoStorageAccess(e) => format!("the key store is locked or can't be reached ({e})"),
        keyring::Error::PlatformFailure(e) => format!("the key store failed ({e})"),
        keyring::Error::NoDefaultStore => "there is no key store".into(),
        _ => "the key store can't read or keep the key".into(),
    })
}
