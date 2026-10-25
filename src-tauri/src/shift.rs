//! Reads whether Shift is held at the moment a file is dropped on MyFile.
//!
//! The drop event from the OS doesn't say which keys were down, and the web
//! view gets no key events during an OS drag, so we ask the OS directly.
//! Both calls read the current keyboard modifier state only. They don't hook
//! input and need no special permissions.

#[cfg(windows)]
pub fn shift_held() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(vkey: i32) -> i16;
    }
    const VK_SHIFT: i32 = 0x10;
    // The high bit is set while the key is down.
    unsafe { (GetAsyncKeyState(VK_SHIFT) as u16 & 0x8000) != 0 }
}

#[cfg(target_os = "macos")]
pub fn shift_held() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceFlagsState(state_id: i32) -> u64;
    }
    const COMBINED_SESSION_STATE: i32 = 0; // kCGEventSourceStateCombinedSessionState
    const FLAG_MASK_SHIFT: u64 = 0x0002_0000; // kCGEventFlagMaskShift
    unsafe { CGEventSourceFlagsState(COMBINED_SESSION_STATE) & FLAG_MASK_SHIFT != 0 }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn shift_held() -> bool {
    false
}
