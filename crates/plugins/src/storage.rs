//! What a plug-in keeps between runs, from plug-in API level 2 on: **settings**, a few named values
//! (the `settings` permission), and a **cache** of named entries the user can clear (the `cache`
//! permission). A plug-in reads and writes them through the host imports ([`crate::host::IMPORTS`]),
//! naming each by a key ([`valid_name`]), never by a path; the host keeps them per plug-in id
//! ([`Storage`], given with [`crate::registry::set_storage`]), so one plug-in can't reach another's.
//!
//! The caps are VectorCraft's own values: [`MAX_SETTINGS`] keys of at most [`MAX_SETTING_BYTES`]
//! each in a settings file of at most [`MAX_SETTINGS_BYTES`], and cache entries of at most
//! [`MAX_CACHE_ENTRY`] bytes, [`MAX_CACHE_BYTES`] and [`MAX_CACHE_ENTRIES`] in all. A write that
//! would pass one fails with [`StorageError::Full`] and changes nothing.

/// Longest key or cache entry name, in bytes.
pub const MAX_NAME: usize = 64;

/// Most settings a plug-in keeps.
pub const MAX_SETTINGS: usize = 256;

/// Largest value of one setting, in bytes.
pub const MAX_SETTING_BYTES: usize = 64 << 10;

/// Largest settings file, in bytes: the keys and the values in base64, as JSON.
pub const MAX_SETTINGS_BYTES: usize = 1 << 20;

/// Largest cache entry, in bytes.
pub const MAX_CACHE_ENTRY: usize = 64 << 20;

/// Most bytes a plug-in's cache holds, all entries together.
pub const MAX_CACHE_BYTES: u64 = 512 << 20;

/// Most entries a plug-in's cache holds.
pub const MAX_CACHE_ENTRIES: usize = 4096;

/// Why a storage call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageError {
    /// The write would pass a cap; nothing was changed.
    Full,
    /// There is nowhere to keep anything: no plug-in store is attached, as in the web build.
    Unavailable,
    /// Reading or writing the file failed, with the system's reason.
    Io(String),
    /// The call would read or write more bytes than its [`Meter`] has left; nothing was changed.
    Budget,
}

/// The bytes a storage call may still read and write, and those it has. The storage charges each
/// file it reads or writes, at its size, before it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meter {
    left: u64,
    used: u64,
    refused: u64,
    /// Past `free` bytes in all, a read or write happens only before the deadline `until` and
    /// Cancel ([`vectorcraft_doc::cancel`]); `stopped` records which refused one.
    free: u64,
    until: Option<web_time::Instant>,
    stopped: Option<Gate>,
}

/// What made a gated [`Meter`] refuse a read or write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    /// The run was canceled.
    Cancel,
    /// The run's deadline passed.
    Deadline,
}

impl Meter {
    /// A meter that lets a call read and write `left` bytes.
    pub fn new(left: u64) -> Self {
        Meter { left, used: 0, refused: 0, free: u64::MAX, until: None, stopped: None }
    }

    /// A meter that lets a call read and write `left` bytes, those past the first `free` only
    /// before `until` and Cancel.
    pub(crate) fn gated(left: u64, free: u64, until: web_time::Instant) -> Self {
        Meter { free, until: Some(until), ..Meter::new(left) }
    }

    /// A meter with no limit, for callers that don't count.
    pub fn unlimited() -> Self {
        Meter::new(u64::MAX)
    }

    /// Counts `bytes` about to be read or written → [`StorageError::Budget`], counting nothing,
    /// when that is more than is left, or when they pass the free bytes of a gated meter after its
    /// deadline or Cancel.
    pub fn take(&mut self, bytes: u64) -> Result<(), StorageError> {
        if bytes > self.left {
            self.refused = bytes;
            return Err(StorageError::Budget);
        }
        if self.used.saturating_add(bytes) > self.free {
            // The deadline first, as the host checks it between two slices.
            let closed = if self.until.is_some_and(|at| web_time::Instant::now() >= at) {
                Some(Gate::Deadline)
            } else {
                vectorcraft_doc::cancel::is_cancelled().then_some(Gate::Cancel)
            };
            if closed.is_some() {
                self.stopped = closed;
                self.refused = bytes;
                return Err(StorageError::Budget);
            }
        }
        self.left -= bytes;
        self.used = self.used.saturating_add(bytes);
        Ok(())
    }

    /// What made the meter refuse a read or write past its free bytes ([`Meter::gated`]), if
    /// anything did.
    pub(crate) fn stopped(&self) -> Option<Gate> {
        self.stopped
    }

    /// The bytes counted so far.
    pub fn used(&self) -> u64 {
        self.used
    }

    /// The bytes of the last read or write the meter refused (0 when it refused none).
    pub(crate) fn refused(&self) -> u64 {
        self.refused
    }
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Full => write!(f, "the plug-in's storage is full"),
            StorageError::Unavailable => write!(f, "no plug-in storage is available"),
            StorageError::Io(why) => write!(f, "{why}"),
            StorageError::Budget => write!(f, "the run has used up its storage bytes"),
        }
    }
}

/// Where one plug-in's settings and cache are kept. Every key and name the host passes is a
/// [`valid_name`], and every value is within its cap ([`MAX_SETTING_BYTES`], [`MAX_CACHE_ENTRY`]);
/// the storage enforces the totals. The calls that read or write a file charge `meter` the bytes
/// they read and write ([`Meter::take`]) before they do: a settings call the whole settings file
/// when it reads the file and the whole file when it writes it, a cache call the entry it reads or
/// writes. A settings call that the storage answers from memory charges nothing.
pub trait Storage: Send + Sync {
    /// The value of setting `key`, if it has one.
    fn setting(&self, key: &str, meter: &mut Meter) -> Result<Option<Vec<u8>>, StorageError>;
    /// Sets `key` to `value`.
    fn set_setting(&self, key: &str, value: &[u8], meter: &mut Meter) -> Result<(), StorageError>;
    /// Removes setting `key` → whether it had a value.
    fn delete_setting(&self, key: &str, meter: &mut Meter) -> Result<bool, StorageError>;
    /// The cache entry `name`, if there is one.
    fn cache_entry(&self, name: &str, meter: &mut Meter) -> Result<Option<Vec<u8>>, StorageError>;
    /// Writes the cache entry `name`.
    fn set_cache_entry(&self, name: &str, value: &[u8], meter: &mut Meter) -> Result<(), StorageError>;
    /// Removes the cache entry `name` → whether there was one.
    fn remove_cache_entry(&self, name: &str) -> Result<bool, StorageError>;
    /// Removes every cache entry → the bytes they held.
    fn clear_cache(&self) -> Result<u64, StorageError>;
    /// The bytes the cache holds now.
    fn cache_bytes(&self) -> u64;
}

/// Is `name` a key or cache entry name a plug-in may use: 1 to [`MAX_NAME`] characters from
/// `A-Z a-z 0-9 _ -`, and not a name Windows keeps for a device (`CON`, `PRN`, `AUX`, `NUL`,
/// `COM0` to `COM9`, `LPT0` to `LPT9`, in any case)? Keys differ in case: `a` and `A` are two.
pub fn valid_name(name: &str) -> bool {
    (1..=MAX_NAME).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) && !device_name(name)
}

/// Is `name` one of the device names Windows keeps?
fn device_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    match upper.as_bytes() {
        [a, b, c] => matches!(&[*a, *b, *c], b"CON" | b"PRN" | b"AUX" | b"NUL"),
        [a, b, c, d] => matches!(&[*a, *b, *c], b"COM" | b"LPT") && d.is_ascii_digit(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_short_plain_and_never_a_device() {
        for ok in ["a", "Z", "zoom-level", "tile_12", &"x".repeat(MAX_NAME), "CONSOLE", "COM10", "LPTX", "con1"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "..", ".", "a/b", "a\\b", "a.b", "a b", "a\u{0}", "\u{7}", "é", &"x".repeat(MAX_NAME + 1), "CON", "con", "Nul", "aux", "prn"]
        {
            assert!(!valid_name(bad), "{bad:?}");
        }
        for n in 0..10 {
            assert!(!valid_name(&format!("COM{n}")) && !valid_name(&format!("lpt{n}")));
        }
    }
}
