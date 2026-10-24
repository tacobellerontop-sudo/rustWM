//! Start rustWM at login through the per-user `Run` registry key.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::w;

const RUN_KEY: windows::core::PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE: windows::core::PCWSTR = w!("rustWM");

fn check(err: windows::Win32::Foundation::WIN32_ERROR) -> std::io::Result<()> {
    if err == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(err.0 as i32))
    }
}

/// Registers `command_line` to run at login.
pub fn enable(command_line: &str) -> std::io::Result<()> {
    let data: Vec<u16> = command_line.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `data` is a NUL-terminated UTF-16 string of the stated size.
    check(unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    })
}

/// Removes the login entry. Succeeds if there was none.
pub fn disable() -> std::io::Result<()> {
    // SAFETY: static key and value names.
    let err = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) };
    if err == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(err)
}

/// The registered command line, if autostart is on.
pub fn current() -> Option<String> {
    let mut buf = [0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its byte size are passed together.
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len]))
}
