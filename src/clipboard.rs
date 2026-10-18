//! The clipboard, for the one action that hands a command to the user instead
//! of running it: the DeepSeek session is pasted into luvus, so the tray only
//! has to leave it there.
//!
//! CF_UNICODETEXT rather than `clip.exe`: the command is ASCII today, but the
//! user's profile directory is not guaranteed to be, and a shell round-trip
//! would mangle it. The buffer is a GMEM_MOVEABLE block, which is what the
//! clipboard takes ownership of.

use std::time::Duration;
use windows_sys::Win32::Foundation::HGLOBAL;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

#[link(name = "kernel32")]
unsafe extern "system" {
    // windows-sys 0.61 exposes the rest of the Global* family but not this one.
    fn GlobalFree(hmem: HGLOBAL) -> HGLOBAL;
}

/// Copies `text` as a NUL-terminated UTF-16 string.
pub fn copy(text: &str) -> Result<(), String> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * std::mem::size_of::<u16>();

    // Another process can hold the clipboard for a few milliseconds; the tray
    // is doing this on a click, so a short wait is invisible.
    let mut opened = false;
    for _ in 0..5 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            opened = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !opened {
        return Err("another program is holding the clipboard".into());
    }

    let result = write(&wide, bytes);
    unsafe { CloseClipboard() };
    result
}

fn write(wide: &[u16], bytes: usize) -> Result<(), String> {
    unsafe {
        if EmptyClipboard() == 0 {
            return Err("could not empty the clipboard".into());
        }
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if handle.is_null() {
            return Err("could not allocate the clipboard buffer".into());
        }
        let target = GlobalLock(handle) as *mut u16;
        if target.is_null() {
            GlobalFree(handle);
            return Err("could not lock the clipboard buffer".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
        GlobalUnlock(handle);
        if SetClipboardData(CF_UNICODETEXT as u32, handle).is_null() {
            GlobalFree(handle);
            return Err("could not set the clipboard".into());
        }
        // On success the clipboard owns the memory; it must not be freed.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "writes to the real clipboard"]
    fn copies_unicode_text() {
        copy("cswap-tray clipboard ✓").unwrap();
    }
}
