use crate::desktop::window_hwnd;
use gpui::Window;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WAIT_ABANDONED, WAIT_OBJECT_0, WPARAM},
    System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject, INFINITE},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const LAUNCH_MUTEX: *const u16 = windows_sys::w!(r"Local\SoloClipboard.Launch");
const INSTANCE_WINDOW: *const u16 = windows_sys::w!("SoloClipboard.MainWindow");
const REOPEN_WINDOW: u32 = WM_APP + 102;

// Hold the mutex from the existing-window check through registration of the new window.
pub struct LaunchLock(OwnedHandle);

impl LaunchLock {
    pub fn acquire() -> io::Result<Self> {
        unsafe {
            let mutex = CreateMutexW(ptr::null(), 0, LAUNCH_MUTEX);
            if mutex.is_null() {
                return Err(io::Error::last_os_error());
            }
            let handle = OwnedHandle::from_raw_handle(mutex);
            match WaitForSingleObject(mutex, INFINITE) {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self(handle)),
                _ => Err(io::Error::last_os_error()),
            }
        }
    }

    pub fn reuse_window(&self, reopen: bool) -> io::Result<bool> {
        let mut hwnd: HWND = ptr::null_mut();
        unsafe {
            if EnumWindows(Some(find_window), (&raw mut hwnd) as LPARAM) == 0 {
                return Err(io::Error::last_os_error());
            }
            if hwnd.is_null() {
                return Ok(false);
            }
            if reopen {
                let mut process = 0;
                GetWindowThreadProcessId(hwnd, &mut process);
                AllowSetForegroundWindow(process);
                if PostMessageW(hwnd, REOPEN_WINDOW, 0, 0) == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        Ok(true)
    }
}

impl Drop for LaunchLock {
    fn drop(&mut self) {
        unsafe { ReleaseMutex(self.0.as_raw_handle()) };
    }
}

unsafe extern "system" fn find_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    unsafe {
        if !GetPropW(hwnd, INSTANCE_WINDOW).is_null() {
            *(lparam as *mut HWND) = hwnd;
        }
    }
    1
}

pub fn register_window(window: &Window) -> io::Result<()> {
    let hwnd = window_hwnd(window).map_err(|error| io::Error::other(error.to_string()))?;
    unsafe {
        if SetWindowSubclass(hwnd, Some(instance_proc), 1, 0) == 0 {
            return Err(io::Error::last_os_error());
        }
        if SetPropW(hwnd, INSTANCE_WINDOW, hwnd) == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

unsafe extern "system" fn instance_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        match message {
            REOPEN_WINDOW => {
                ShowWindow(hwnd, SW_MAXIMIZE);
                SetForegroundWindow(hwnd);
                return 0;
            }
            WM_NCDESTROY => {
                RemovePropW(hwnd, INSTANCE_WINDOW);
                RemoveWindowSubclass(hwnd, Some(instance_proc), subclass_id);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}
