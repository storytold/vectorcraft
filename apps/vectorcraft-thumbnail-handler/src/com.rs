//! The COM server: the thumbnail provider, its class factory and the functions the DLL exports
//! (`DllGetClassObject`, `DllCanUnloadNow`, `DllRegisterServer` and `DllUnregisterServer`).
//!
//! COM, GDI and the registry are C APIs with no safe binding, hence the scoped `unsafe_code`
//! allowance.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::io::{self, Read};
use std::os::windows::ffi::OsStringExt;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, ERROR_ALREADY_INITIALIZED, ERROR_FILE_NOT_FOUND, HINSTANCE, HMODULE,
    S_FALSE, S_OK,
};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP};
use windows::Win32::System::Com::{CoTaskMemFree, IClassFactory, IClassFactory_Impl, IStream};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithStream, IInitializeWithStream_Impl};
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppDataLow, IThumbnailProvider, IThumbnailProvider_Impl, KNOWN_FOLDER_FLAG, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify,
    SHGetKnownFolderPath, WTS_ALPHATYPE, WTSAT_ARGB,
};
use windows::core::{BOOL, Error, GUID, HRESULT, HSTRING, IUnknown, Interface, PCWSTR, Ref, Result, implement};

use crate::{CLI, CLSID_VALUE, DEADLINE, Document, MAX_FILE, Picture, RegistryValue};

/// The handler's class id ([`crate::CLSID`]).
const CLASS_ID: GUID = GUID::from_u128(CLSID_VALUE);
/// The registry key the values of [`crate::registry_values`] go under.
const CLASSES: &str = "Software\\Classes";
/// The longest DLL path read, in UTF-16 units (Windows' own limit).
const MAX_PATH: usize = 32_767;

/// The DLL's module handle, from `DllMain`.
static MODULE: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
/// Providers and class factories alive plus `LockServer` locks held; the DLL can be unloaded at 0.
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// The thumbnail provider: Explorer hands it the file's stream, then asks for the picture.
#[implement(IInitializeWithStream, IThumbnailProvider)]
struct Provider {
    document: Mutex<Option<Document<StreamReader>>>,
}

impl Provider {
    fn new() -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Self { document: Mutex::new(None) }
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

impl IInitializeWithStream_Impl for Provider_Impl {
    fn Initialize(&self, stream: Ref<IStream>, _mode: u32) -> Result<()> {
        let stream = stream.ok()?.clone();
        let mut slot = self.document.lock().map_err(|_| Error::from(E_FAIL))?;
        if slot.is_some() {
            return Err(ERROR_ALREADY_INITIALIZED.to_hresult().into());
        }
        *slot = Some(Document::new(StreamReader(stream)));
        Ok(())
    }
}

impl IThumbnailProvider_Impl for Provider_Impl {
    fn GetThumbnail(&self, cx: u32, bitmap: *mut HBITMAP, alpha: *mut WTS_ALPHATYPE) -> Result<()> {
        if bitmap.is_null() || alpha.is_null() {
            return Err(E_POINTER.into());
        }
        let mut slot = self.document.lock().map_err(|_| Error::from(E_FAIL))?;
        let document = slot.as_mut().ok_or_else(|| Error::new(E_FAIL, "not initialized"))?;
        // A panic must not unwind into Explorer's process.
        let picture = match std::panic::catch_unwind(AssertUnwindSafe(|| thumbnail(document, cx))) {
            Ok(Ok(picture)) => picture,
            Ok(Err(e)) => return Err(Error::new(E_FAIL, e)),
            Err(_) => return Err(Error::new(E_FAIL, "the thumbnail could not be made")),
        };
        drop(slot);
        let made = dib_section(&picture)?;
        // SAFETY: both pointers were checked for null above, and Explorer passes them writable.
        unsafe {
            *bitmap = made;
            *alpha = WTSAT_ARGB;
        }
        Ok(())
    }
}

/// The class factory COM asks for the provider.
#[implement(IClassFactory)]
struct Factory;

impl Factory {
    fn new() -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, iid: *const GUID, object: *mut *mut c_void) -> Result<()> {
        if iid.is_null() || object.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: `object` was checked for null above.
        unsafe { *object = null_mut() };
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let provider: IUnknown = Provider::new().into();
        // SAFETY: both pointers were checked for null above.
        unsafe { provider.query(iid, object) }.ok()
    }

    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            LIVE.fetch_add(1, Ordering::SeqCst);
        } else {
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// `IStream` as [`Read`].
struct StreamReader(IStream);

impl Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let mut read = 0u32;
        // SAFETY: `buf` holds at least `len` writable bytes, and `read` outlives the call.
        unsafe { self.0.Read(buf.as_mut_ptr().cast(), len, Some(&mut read)) }.ok().map_err(io::Error::other)?;
        usize::try_from(read).map_err(io::Error::other)
    }
}

/// The picture of `document`, at most `cx` pixels on its longer side.
fn thumbnail(document: &mut Document<StreamReader>, cx: u32) -> std::result::Result<Picture, String> {
    let size = crate::request_size(cx).ok_or("a thumbnail of 0 pixels")?;
    let bytes = document.bytes(MAX_FILE).map_err(|e| e.to_string())?;
    let dll = module_path()?;
    let cli = PathBuf::from(dll).parent().ok_or("the DLL's folder is unknown")?.join(CLI);
    let png = crate::render(&cli, bytes, size, &staging_folder(), DEADLINE)?;
    crate::picture(&png, size)
}

/// Creates the thumbnail staging folder in LocalLow, falling back to the temp folder.
fn staging_folder() -> PathBuf {
    // SAFETY: the folder ID is valid, and no token means the current user.
    let Ok(folder) = (unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppDataLow, KNOWN_FOLDER_FLAG(0), None) }) else {
        return std::env::temp_dir();
    };
    // SAFETY: the call returned a NUL-terminated path allocated by COM; copy it before freeing it.
    let path = unsafe {
        let path = PathBuf::from(std::ffi::OsString::from_wide(folder.as_wide()));
        CoTaskMemFree(Some(folder.0.cast()));
        path
    };
    let path = path.join("VectorCraft").join("Thumbnails");
    if std::fs::create_dir_all(&path).is_ok() { path } else { std::env::temp_dir() }
}

/// `picture` as a top-down 32-bit DIB section.
fn dib_section(picture: &Picture) -> Result<HBITMAP> {
    let bad = || Error::new(E_FAIL, "the picture's size is wrong");
    let width = i32::try_from(picture.width).map_err(|_| bad())?;
    let height = i32::try_from(picture.height).map_err(|_| bad())?;
    let len = usize::try_from(u64::from(picture.width) * u64::from(picture.height) * 4).map_err(|_| bad())?;
    if picture.bgra.len() != len {
        return Err(bad());
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // A negative height: the rows go from top to bottom.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = null_mut();
    // SAFETY: `info` describes a 32-bit picture and `bits` outlives the call.
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }?;
    if bits.is_null() {
        // SAFETY: `bitmap` was just made and nothing else holds it.
        unsafe {
            let _ = DeleteObject(bitmap.into());
        }
        return Err(Error::new(E_FAIL, "the bitmap has no pixels"));
    }
    // SAFETY: the DIB section holds `width * height * 4` bytes, `len`, and `bgra` holds `len` bytes.
    unsafe { std::ptr::copy_nonoverlapping(picture.bgra.as_ptr(), bits.cast::<u8>(), len) };
    Ok(bitmap)
}

/// The path of this DLL.
fn module_path() -> std::result::Result<String, String> {
    let module = HMODULE(MODULE.load(Ordering::SeqCst));
    if module.0.is_null() {
        return Err("the DLL's module handle is unknown".into());
    }
    let mut buf = vec![0u16; MAX_PATH];
    // SAFETY: `module` is this DLL's handle, which stays valid while the DLL is loaded.
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buf) };
    let path = usize::try_from(len).ok().filter(|&n| n > 0 && n < buf.len()).and_then(|n| buf.get(..n)).ok_or("the DLL's path is unknown")?;
    String::from_utf16(path).map_err(|e| e.to_string())
}

/// Writes `value` under `Software\Classes` of the current user.
fn set(value: &RegistryValue) -> Result<()> {
    let key = HSTRING::from(format!("{CLASSES}\\{}", value.key));
    let name = value.name.map(HSTRING::from);
    let name = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
    let data: Vec<u16> = value.data.encode_utf16().chain([0]).collect();
    let bytes = u32::try_from(data.len() * 2).map_err(|_| Error::from(E_FAIL))?;
    // SAFETY: `data` holds `bytes` bytes of a NUL-terminated string, and the names outlive the call.
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &key, name, REG_SZ.0, Some(data.as_ptr().cast()), bytes) }.ok()
}

/// The default value of `key` under `Software\Classes` of the current user, when it is a string.
fn default_value(key: &str) -> Option<String> {
    let key = HSTRING::from(format!("{CLASSES}\\{key}"));
    let mut buf = [0u16; 128];
    let mut bytes = u32::try_from(buf.len() * 2).ok()?;
    // SAFETY: `buf` holds `bytes` writable bytes, and `bytes` outlives the call.
    unsafe { RegGetValueW(HKEY_CURRENT_USER, &key, PCWSTR::null(), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut bytes)) }
        .ok()
        .ok()?;
    let units = buf.get(..usize::try_from(bytes / 2).ok()?)?;
    String::from_utf16(units).ok().map(|s| s.trim_end_matches('\0').to_owned())
}

/// Removes `key` under `Software\Classes` of the current user with its subkeys; a key that isn't
/// there is no error.
fn remove(key: &str) -> Result<()> {
    let key = HSTRING::from(format!("{CLASSES}\\{key}"));
    // SAFETY: `key` outlives the call.
    match unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &key) } {
        ERROR_FILE_NOT_FOUND => Ok(()),
        e => e.ok(),
    }
}

/// Tells Explorer that file associations changed, so it asks for thumbnails again.
fn associations_changed() {
    // SAFETY: SHCNE_ASSOCCHANGED takes no items.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Registers the handler for the current user ([`crate::registry_values`]).
fn register() -> Result<()> {
    let dll = module_path().map_err(|e| Error::new(E_FAIL, e))?;
    crate::registry_values(&dll).iter().try_for_each(set)
}

/// Removes the class key and each extension's thumbnail handler key that names this handler.
fn unregister() -> Result<()> {
    for ext in crate::EXTENSIONS {
        let key = crate::shell_ex_key(ext);
        if default_value(&key).is_some_and(|v| crate::names_this_handler(&v)) {
            remove(&key)?;
        }
    }
    remove(&crate::class_key())
}

/// Keeps the DLL's module handle for [`module_path`].
#[unsafe(no_mangle)]
extern "system" fn DllMain(module: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(module.0, Ordering::SeqCst);
    }
    true.into()
}

/// COM's request for the class factory of class `class`.
///
/// # Safety
///
/// `class` and `iid` point to GUIDs and `object` to writable memory for an interface pointer, as
/// COM passes them.
#[unsafe(no_mangle)]
unsafe extern "system" fn DllGetClassObject(class: *const GUID, iid: *const GUID, object: *mut *mut c_void) -> HRESULT {
    if class.is_null() || iid.is_null() || object.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller passes valid pointers, checked for null above.
    unsafe {
        *object = null_mut();
        if *class != CLASS_ID {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory::new().into();
        factory.query(iid, object)
    }
}

/// Whether COM may unload the DLL: no provider or class factory is alive and no lock is held.
#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LIVE.load(Ordering::SeqCst) == 0 { S_OK } else { S_FALSE }
}

/// `regsvr32`: registers the handler for the current user.
#[unsafe(no_mangle)]
extern "system" fn DllRegisterServer() -> HRESULT {
    let result = register();
    if result.is_err() {
        let _ = unregister();
    }
    associations_changed();
    result.map_or_else(|e| e.code(), |()| S_OK)
}

/// `regsvr32 /u`: removes the current user's registration.
#[unsafe(no_mangle)]
extern "system" fn DllUnregisterServer() -> HRESULT {
    let result = unregister();
    associations_changed();
    result.map_or_else(|e| e.code(), |()| S_OK)
}
