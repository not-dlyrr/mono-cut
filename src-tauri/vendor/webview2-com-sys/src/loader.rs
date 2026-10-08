//! Source-built WebView2 loader. No Microsoft WebView2 SDK binary is linked or copied.
//!
//! Runtime discovery and the internal entry point follow the openly licensed
//! implementations cited in README.md. The WebView2 runtime itself is supplied
//! by the operating system/user, never bundled by Mono Cut.

#![allow(non_snake_case)]

use std::{collections::BTreeMap, ffi::{c_void, OsStr, OsString}, os::windows::ffi::{OsStrExt, OsStringExt}, path::{Path, PathBuf}, ptr, sync::{Mutex, OnceLock}};
use windows_core::{HRESULT, PCWSTR, PWSTR};

const S_OK: HRESULT = HRESULT(0);
const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);
const E_INVALIDARG: HRESULT = HRESULT(0x80070057u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x80004001u32 as i32);
const E_OUTOFMEMORY: HRESULT = HRESULT(0x8007000eu32 as i32);
const E_FAIL: HRESULT = HRESULT(0x80004005u32 as i32);
const CLIENT_KEY: &str = "Software\\Microsoft\\EdgeUpdate\\ClientState\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
const MINIMUM_VERSION: [u32; 4] = [86, 0, 616, 0];

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(key: *mut c_void, name: *const u16, options: u32, access: u32, result: *mut *mut c_void) -> i32;
    fn RegQueryValueExW(key: *mut c_void, name: *const u16, reserved: *mut u32, kind: *mut u32, data: *mut u8, bytes: *mut u32) -> i32;
    fn RegCloseKey(key: *mut c_void) -> i32;
}
#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(path: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
    fn GetLastError() -> u32;
    fn ExpandEnvironmentStringsW(source: *const u16, target: *mut u16, size: u32) -> u32;
}
#[link(name = "ole32")]
extern "system" { fn CoTaskMemAlloc(bytes: usize) -> *mut c_void; }
#[link(name = "version")]
extern "system" {
    fn GetFileVersionInfoSizeW(path: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(path: *const u16, handle: u32, size: u32, data: *mut c_void) -> i32;
    fn VerQueryValueW(data: *const c_void, key: *const u16, value: *mut *mut c_void, size: *mut u32) -> i32;
}

fn wide(value: &OsStr) -> Vec<u16> { value.encode_wide().chain(Some(0)).collect() }
fn hresult(error: u32) -> HRESULT { if error == 0 { E_FAIL } else { HRESULT((0x80070000u32 | (error & 0xffff)) as i32) } }

unsafe fn read_wide(value: PCWSTR) -> Option<Vec<u16>> {
    if value.0.is_null() { return None; }
    for len in 0..32768 { if *value.0.add(len) == 0 { return Some(std::slice::from_raw_parts(value.0, len).to_vec()); } }
    None
}

fn parse_version(value: &str) -> Option<[u32; 4]> {
    let numeric = value.split_whitespace().next()?;
    let mut result = [0; 4];
    let parts: Vec<_> = numeric.split('.').collect();
    if parts.is_empty() || parts.len() > 4 { return None; }
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) { return None; }
        result[i] = part.parse().ok()?;
    }
    Some(result)
}

unsafe fn registry_folder(root: *mut c_void, view: u32) -> Option<PathBuf> {
    let name = wide(OsStr::new(CLIENT_KEY)); let value_name = wide(OsStr::new("EBWebView"));
    let mut key = ptr::null_mut();
    if RegOpenKeyExW(root, name.as_ptr(), 0, 0x20019 | view, &mut key) != 0 { return None; }
    let result = (|| {
        let mut bytes = 0; let mut kind = 0;
        if RegQueryValueExW(key, value_name.as_ptr(), ptr::null_mut(), &mut kind, ptr::null_mut(), &mut bytes) != 0 || !matches!(kind, 1 | 2) || bytes == 0 || bytes > 65536 { return None; }
        let mut text = vec![0u16; bytes as usize / 2 + 1];
        if RegQueryValueExW(key, value_name.as_ptr(), ptr::null_mut(), &mut kind, text.as_mut_ptr().cast(), &mut bytes) != 0 { return None; }
        text.truncate(text.iter().position(|&c| c == 0).unwrap_or(text.len()));
        if kind == 2 {
            text.push(0);
            let length = ExpandEnvironmentStringsW(text.as_ptr(), ptr::null_mut(), 0);
            if length == 0 || length > 32768 { return None; }
            let mut expanded = vec![0u16; length as usize];
            if ExpandEnvironmentStringsW(text.as_ptr(), expanded.as_mut_ptr(), length) != length { return None; }
            expanded.pop(); text = expanded;
        }
        if text.is_empty() { None } else { Some(PathBuf::from(OsString::from_wide(&text))) }
    })();
    RegCloseKey(key); result
}

fn client_path(folder: &Path) -> PathBuf {
    let architecture = if cfg!(target_arch = "x86_64") { "x64" } else if cfg!(target_arch = "aarch64") { "arm64" } else { "x86" };
    folder.join("EBWebView").join(architecture).join("EmbeddedBrowserWebView.dll")
}

#[repr(C)]
struct FixedFileInfo { signature: u32, structure_version: u32, file_ms: u32, file_ls: u32, product_ms: u32, product_ls: u32, flags_mask: u32, flags: u32, os: u32, kind: u32, subtype: u32, date_ms: u32, date_ls: u32 }

unsafe fn file_version(path: &Path) -> Option<String> {
    let filename = wide(path.as_os_str()); let mut handle = 0;
    let length = GetFileVersionInfoSizeW(filename.as_ptr(), &mut handle);
    if length == 0 || length > 16 * 1024 * 1024 { return None; }
    let mut data = vec![0u8; length as usize];
    if GetFileVersionInfoW(filename.as_ptr(), handle, length, data.as_mut_ptr().cast()) == 0 { return None; }
    let mut value = ptr::null_mut(); let mut bytes = 0; let key = wide(OsStr::new("\\"));
    if VerQueryValueW(data.as_ptr().cast(), key.as_ptr(), &mut value, &mut bytes) == 0 || value.is_null() || bytes < std::mem::size_of::<FixedFileInfo>() as u32 { return None; }
    let fixed = ptr::read_unaligned(value.cast::<FixedFileInfo>());
    if fixed.signature != 0xfeef04bd { return None; }
    Some(format!("{}.{}.{}.{}", fixed.product_ms >> 16, fixed.product_ms & 0xffff, fixed.product_ls >> 16, fixed.product_ls & 0xffff))
}

struct Client { path: PathBuf, version: String, runtime_type: u32 }
unsafe fn find_client(folder: PCWSTR) -> Result<Client, HRESULT> {
    let requested = std::env::var_os("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER").filter(|v| !v.is_empty())
        .or_else(|| read_wide(folder).filter(|v| !v.is_empty()).map(|v| OsString::from_wide(&v)));
    if let Some(directory) = requested {
        let directory = PathBuf::from(directory);
        if !directory.is_absolute() { return Err(E_INVALIDARG); }
        let path = client_path(&directory);
        let version = file_version(&path).ok_or_else(|| hresult(2))?;
        if parse_version(&version).ok_or(E_INVALIDARG)? < MINIMUM_VERSION { return Err(hresult(1150)); }
        return Ok(Client { path, version, runtime_type: 1 });
    }
    // Win32 predefined handles must be sign extended on 64-bit systems.
    for root in [0x80000001u32, 0x80000002u32] {
        for view in [0x200, 0x100] {
            let Some(directory) = registry_folder(root as i32 as isize as *mut c_void, view) else { continue; };
            if !directory.is_absolute() { continue; }
            let path = client_path(&directory);
            if !path.is_file() { continue; }
            let version = directory.file_name().and_then(|v| v.to_str()).filter(|v| parse_version(v).is_some()).map(str::to_owned).or_else(|| file_version(&path));
            let Some(version) = version else { continue; };
            if parse_version(&version).is_some_and(|v| v >= MINIMUM_VERSION) { return Ok(Client { path, version, runtime_type: 0 }); }
        }
    }
    Err(hresult(2))
}

// Keep the client DLL resident while its COM objects and asynchronous callbacks
// are alive. One handle per runtime path avoids repeated LoadLibrary references.
static MODULES: OnceLock<Mutex<BTreeMap<PathBuf, usize>>> = OnceLock::new();
unsafe fn client_entry(path: &Path) -> Result<*mut c_void, HRESULT> {
    let mut modules = MODULES.get_or_init(|| Mutex::new(BTreeMap::new())).lock().map_err(|_| E_FAIL)?;
    let module = if let Some(&module) = modules.get(path) { module as *mut c_void } else {
        let filename = wide(path.as_os_str());
        // Resolve dependencies from the absolute runtime DLL directory and trusted system paths.
        let module = LoadLibraryExW(filename.as_ptr(), ptr::null_mut(), 0x100 | 0x1000);
        if module.is_null() { return Err(hresult(GetLastError())); }
        let entry = GetProcAddress(module, b"CreateWebViewEnvironmentWithOptionsInternal\0".as_ptr());
        if entry.is_null() { let error = hresult(GetLastError()); FreeLibrary(module); return Err(error); }
        modules.insert(path.to_owned(), module as usize); module
    };
    let entry = GetProcAddress(module, b"CreateWebViewEnvironmentWithOptionsInternal\0".as_ptr());
    if entry.is_null() { Err(hresult(GetLastError())) } else { Ok(entry) }
}

pub unsafe extern "system" fn CreateCoreWebView2EnvironmentWithOptions(folder: PCWSTR, user_data: PCWSTR, options: *mut c_void, handler: *mut c_void) -> HRESULT {
    if handler.is_null() { return E_POINTER; }
    let client = match find_client(folder) { Ok(client) => client, Err(error) => return error };
    let entry = match client_entry(&client.path) { Ok(entry) => entry, Err(error) => return error };
    type CreateEnvironment = unsafe extern "system" fn(bool, u32, PCWSTR, *mut c_void, *mut c_void) -> HRESULT;
    let create: CreateEnvironment = std::mem::transmute(entry);
    let overridden_user_data = std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").map(|v| wide(&v));
    create(true, client.runtime_type, overridden_user_data.as_ref().map(|v| PCWSTR(v.as_ptr())).unwrap_or(user_data), options, handler)
}

pub unsafe extern "system" fn CreateCoreWebView2Environment(handler: *mut c_void) -> HRESULT {
    CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), PCWSTR::null(), ptr::null_mut(), handler)
}

pub unsafe extern "system" fn GetAvailableCoreWebView2BrowserVersionString(folder: PCWSTR, output: *mut PWSTR) -> HRESULT {
    if output.is_null() { return E_POINTER; }
    *output = PWSTR::null();
    let client = match find_client(folder) { Ok(client) => client, Err(error) => return error };
    let version = wide(OsStr::new(&client.version));
    let memory = CoTaskMemAlloc(version.len() * 2).cast::<u16>();
    if memory.is_null() { return E_OUTOFMEMORY; }
    ptr::copy_nonoverlapping(version.as_ptr(), memory, version.len()); *output = PWSTR(memory); S_OK
}

pub unsafe extern "system" fn GetAvailableCoreWebView2BrowserVersionStringWithOptions(folder: PCWSTR, options: *mut c_void, output: *mut PWSTR) -> HRESULT {
    // Wry does not use this API. Do not silently ignore unsupported channel-selection options.
    if !options.is_null() { if !output.is_null() { *output = PWSTR::null(); } return E_NOTIMPL; }
    GetAvailableCoreWebView2BrowserVersionString(folder, output)
}

pub unsafe extern "system" fn CompareBrowserVersions(first: PCWSTR, second: PCWSTR, output: *mut i32) -> HRESULT {
    if output.is_null() { return E_POINTER; }
    let Some(first) = read_wide(first).and_then(|v| parse_version(&String::from_utf16_lossy(&v))) else { return E_INVALIDARG; };
    let Some(second) = read_wide(second).and_then(|v| parse_version(&String::from_utf16_lossy(&v))) else { return E_INVALIDARG; };
    *output = match first.cmp(&second) { std::cmp::Ordering::Less => -1, std::cmp::Ordering::Equal => 0, std::cmp::Ordering::Greater => 1 }; S_OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_core::w;
    #[test]
    fn versions_compare_numerically_and_ignore_channel_suffix() {
        let mut result = 0;
        assert_eq!(unsafe { CompareBrowserVersions(w!("130.0.10.0 beta"), w!("130.0.9.0"), &mut result) }, S_OK);
        assert_eq!(result, 1);
        assert_eq!(unsafe { CompareBrowserVersions(w!("130.0.10.0"), w!("130.0.10.0 dev"), &mut result) }, S_OK);
        assert_eq!(result, 0);
    }
    #[test]
    fn invalid_versions_and_null_outputs_fail() {
        let mut result = 0;
        assert_eq!(unsafe { CompareBrowserVersions(w!("bad"), w!("130.0.0.0"), &mut result) }, E_INVALIDARG);
        assert_eq!(unsafe { CompareBrowserVersions(w!("130.0.0.0"), w!("130.0.0.0"), ptr::null_mut()) }, E_POINTER);
        assert_eq!(unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), ptr::null_mut()) }, E_POINTER);
        assert_eq!(unsafe { CreateCoreWebView2Environment(ptr::null_mut()) }, E_POINTER);
    }
    #[test]
    fn missing_explicit_runtime_clears_version_output() {
        let directory = tempfile::tempdir().unwrap(); let path = wide(directory.path().as_os_str());
        let mut output = PWSTR(1usize as *mut u16);
        let result = unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR(path.as_ptr()), &mut output) };
        assert!(result.is_err()); assert!(output.0.is_null());
    }
    #[test]
    fn discovers_installed_runtime_when_present() {
        // Installation is a system prerequisite, not a bundled test dependency.
        let Some(path) = (unsafe { [0x80000001u32, 0x80000002u32].into_iter().find_map(|root| registry_folder(root as i32 as isize as *mut c_void, 0x200)) }) else { return; };
        if !client_path(&path).is_file() { return; }
        let client = unsafe { find_client(PCWSTR::null()) }.unwrap();
        assert!(parse_version(&client.version).unwrap() >= MINIMUM_VERSION);
        assert!(!unsafe { client_entry(&client.path) }.unwrap().is_null());
    }
}
