//! Adding rustWM's folder to the per-user `Path`, so `rwm` works in any new
//! terminal without `.\`.

use std::path::Path;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, LPARAM, WPARAM};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_EXPAND_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};
use windows::core::w;

const ENV_KEY: windows::core::PCWSTR = w!("Environment");
const VALUE: windows::core::PCWSTR = w!("Path");

/// The user's `Path` exactly as stored (unexpanded); empty if unset.
fn read() -> std::io::Result<String> {
    // Environment values are limited to 32767 characters.
    let mut buf = vec![0u16; 32768];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its byte size are passed together.
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            ENV_KEY,
            VALUE,
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if err == ERROR_FILE_NOT_FOUND {
        return Ok(String::new());
    }
    if err != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(err.0 as i32));
    }
    let len = (size as usize / 2).saturating_sub(1);
    Ok(String::from_utf16_lossy(&buf[..len]))
}

fn same_dir(entry: &str, dir: &Path) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches('\\').to_lowercase();
    norm(entry) == norm(&dir.to_string_lossy())
}

/// Whether `dir` is in the user's `Path`.
pub fn contains(dir: &Path) -> bool {
    read().is_ok_and(|p| p.split(';').any(|e| same_dir(e, dir)))
}

/// Appends `dir` to the user's `Path` unless it is already there, and tells
/// running programs. Returns false if it was already present.
pub fn add(dir: &Path) -> std::io::Result<bool> {
    let current = read()?;
    if current.split(';').any(|e| same_dir(e, dir)) {
        return Ok(false);
    }
    let mut new = current.trim_end_matches(';').to_string();
    if !new.is_empty() {
        new.push(';');
    }
    new.push_str(&dir.to_string_lossy());
    let data: Vec<u16> = new.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `data` is a NUL-terminated UTF-16 string of the stated size.
    let err = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            ENV_KEY,
            VALUE,
            REG_EXPAND_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    };
    if err != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(err.0 as i32));
    }
    // SAFETY: broadcasting a static string; the timeout bounds the wait.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("Environment").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            2000,
            None,
        );
    }
    Ok(true)
}
