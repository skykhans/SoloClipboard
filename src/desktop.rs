use gpui::{Keystroke, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};
use std::{cell::Cell, io, mem, ptr, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, RegisterHotKey, SendInput, UnregisterHotKey, INPUT, INPUT_0,
            INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
            MOD_SHIFT, MOD_WIN, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_V,
        },
        Shell::{
            DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, Shell_NotifyIconW, NIF_ICON,
            NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
        },
        WindowsAndMessaging::*,
    },
};

const TRAY_MESSAGE: u32 = WM_APP + 100;
const HIDE_MINIMIZED: u32 = WM_APP + 101;
const ICON_RESOURCE: *const u16 = 101 as *const u16;
const OPEN_MENU: usize = 1;
const EXIT_MENU: usize = 2;
const STARTUP_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_VALUE: &str = "SoloClipboard";
const FILE_NOT_FOUND_HRESULT: i32 = 0x80070002_u32 as i32;
pub const STARTUP_ARG: &str = "--startup";

pub fn starts_with_windows() -> Result<bool, String> {
    let result = windows_registry::CURRENT_USER
        .open(STARTUP_KEY)
        .and_then(|key| key.get_type(STARTUP_VALUE));
    match result {
        Ok(_) => Ok(true),
        Err(error) if error.code().0 == FILE_NOT_FOUND_HRESULT => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

pub fn set_start_with_windows(enabled: bool) -> Result<(), String> {
    let mut options = windows_registry::CURRENT_USER.options();
    options.write();
    if enabled {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let exe = exe.to_str().ok_or("程序路径不是有效的 Unicode")?;
        let command = format!("\"{exe}\" {STARTUP_ARG}");
        options
            .create()
            .open(STARTUP_KEY)
            .and_then(|key| key.set_string(STARTUP_VALUE, command))
            .map_err(|error| error.to_string())
    } else {
        match options
            .open(STARTUP_KEY)
            .and_then(|key| key.remove_value(STARTUP_VALUE))
        {
            Err(error) if error.code().0 == FILE_NOT_FOUND_HRESULT => Ok(()),
            result => result.map_err(|error| error.to_string()),
        }
    }
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
pub struct Shortcut {
    modifiers: u32,
    key: u32,
}

impl Shortcut {
    pub fn from_keystroke(stroke: &Keystroke) -> Option<Self> {
        let keys = stroke.key.as_bytes();
        let key = match keys {
            [letter] if letter.is_ascii_alphanumeric() => u32::from(letter.to_ascii_uppercase()),
            [b'f', ..] if keys.len() <= 3 => {
                let number: u32 = stroke.key[1..].parse().ok()?;
                if !(1..=24).contains(&number) || number == 12 {
                    return None;
                }
                0x70 + number - 1
            }
            _ => return None,
        };
        let modifiers = u32::from(stroke.modifiers.control) * MOD_CONTROL
            | u32::from(stroke.modifiers.alt) * MOD_ALT
            | u32::from(stroke.modifiers.shift) * MOD_SHIFT
            | u32::from(stroke.modifiers.platform) * MOD_WIN;
        (modifiers != 0).then_some(Self { modifiers, key })
    }

    pub fn label(self) -> String {
        let mut label = String::new();
        for (flag, name) in [
            (MOD_CONTROL, "Ctrl + "),
            (MOD_ALT, "Alt + "),
            (MOD_SHIFT, "Shift + "),
            (MOD_WIN, "Win + "),
        ] {
            if self.modifiers & flag != 0 {
                label.push_str(name);
            }
        }
        if (0x70..=0x87).contains(&self.key) {
            label.push_str(&format!("F{}", self.key - 0x70 + 1));
        } else if let Some(key) = char::from_u32(self.key) {
            label.push(key);
        }
        label
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
pub struct Settings {
    pub close_to_tray: bool,
    pub shortcut: Option<Shortcut>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            close_to_tray: false,
            shortcut: Some(Shortcut {
                modifiers: MOD_CONTROL | MOD_ALT,
                key: u32::from(b'V'),
            }),
        }
    }
}

pub struct TrayWindow(Rc<WindowState>);

struct WindowState {
    hwnd: Cell<HWND>,
    icon: HICON,
    taskbar_created: u32,
    has_icon: Cell<bool>,
    close_to_tray: Cell<bool>,
    exiting: Cell<bool>,
    hotkey: Cell<Option<(i32, Shortcut)>>,
    paste_target: Cell<HWND>,
}

impl TrayWindow {
    pub fn new(
        window: &Window,
        close_to_tray: bool,
        previous_window: HWND,
    ) -> Result<Self, String> {
        let handle = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("无法获取 Windows 窗口句柄".into());
        };
        let hwnd = handle.hwnd.get() as HWND;
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let icon = LoadImageW(
                instance,
                ICON_RESOURCE,
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_SHARED,
            );
            if icon.is_null() {
                return Err(io::Error::last_os_error().to_string());
            }
            let state = Rc::new(WindowState {
                hwnd: Cell::new(hwnd),
                icon,
                taskbar_created: RegisterWindowMessageW(windows_sys::w!("TaskbarCreated")),
                has_icon: Cell::new(false),
                close_to_tray: Cell::new(close_to_tray),
                exiting: Cell::new(false),
                hotkey: Cell::new(None),
                paste_target: Cell::new(ptr::null_mut()),
            });
            state.remember_paste_target(previous_window);
            if SetWindowSubclass(hwnd, Some(window_proc), 1, Rc::as_ptr(&state) as usize) == 0 {
                return Err(io::Error::last_os_error().to_string());
            }
            let tray = Self(state);
            if !tray.0.add_icon() {
                return Err("无法添加通知区域图标".into());
            }
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_NOACTIVATE as isize);
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as LPARAM);
            let large = LoadIconW(instance, ICON_RESOURCE);
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, large as LPARAM);
            Ok(tray)
        }
    }

    pub fn paste(&self, copy: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        self.0
            .remember_paste_target(unsafe { GetForegroundWindow() });
        let target = self.0.paste_target.get();
        if target.is_null() || unsafe { IsWindow(target) } == 0 {
            return Err("请先在目标应用中定位输入位置，再打开剪贴板".into());
        }
        unsafe {
            if [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
                .iter()
                .any(|key| GetAsyncKeyState(i32::from(*key)) < 0)
            {
                return Err("请松开修饰键后再粘贴".into());
            }
        }
        copy()?;
        unsafe {
            if GetForegroundWindow() != target
                && (SetForegroundWindow(target) == 0
                    || SendMessageTimeoutW(
                        target,
                        WM_NULL,
                        0,
                        0,
                        SMTO_ABORTIFHUNG | SMTO_BLOCK,
                        250,
                        ptr::null_mut(),
                    ) == 0
                    || GetForegroundWindow() != target)
            {
                return Err("无法切换回原来的输入窗口".into());
            }
            let keys = [
                (VK_CONTROL, 0),
                (VK_V, 0),
                (VK_V, KEYEVENTF_KEYUP),
                (VK_CONTROL, KEYEVENTF_KEYUP),
            ]
            .map(|(key, flags)| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: key,
                        dwFlags: flags,
                        ..Default::default()
                    },
                },
            });
            if SendInput(
                keys.len() as u32,
                keys.as_ptr(),
                mem::size_of::<INPUT>() as i32,
            ) != keys.len() as u32
            {
                return Err("无法向原来的输入窗口发送粘贴操作".into());
            }
            ShowWindow(self.0.hwnd.get(), SW_HIDE);
        }
        Ok(())
    }

    pub fn set_close_to_tray(&self, enabled: bool) {
        self.0.close_to_tray.set(enabled);
    }

    pub fn set_shortcut(
        &self,
        shortcut: Option<Shortcut>,
        save: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let previous = self.0.hotkey.get();
        if previous.map(|(_, shortcut)| shortcut) == shortcut {
            return save();
        }
        let hwnd = self.0.hwnd.get();
        let id = if previous.is_some_and(|(id, _)| id == 1) {
            2
        } else {
            1
        };
        if let Some(shortcut) = shortcut {
            if unsafe { RegisterHotKey(hwnd, id, shortcut.modifiers | MOD_NOREPEAT, shortcut.key) }
                == 0
            {
                return Err(format!(
                    "无法注册快捷键 {}：{}",
                    shortcut.label(),
                    io::Error::last_os_error()
                ));
            }
        }
        if let Err(error) = save() {
            if shortcut.is_some() {
                unsafe { UnregisterHotKey(hwnd, id) };
            }
            return Err(error);
        }
        if let Some((id, _)) = previous {
            unsafe { UnregisterHotKey(hwnd, id) };
        }
        self.0.hotkey.set(shortcut.map(|shortcut| (id, shortcut)));
        Ok(())
    }
}

impl Drop for TrayWindow {
    fn drop(&mut self) {
        self.0.detach();
    }
}

impl WindowState {
    fn remember_paste_target(&self, hwnd: HWND) {
        if !hwnd.is_null() {
            let mut process = 0;
            unsafe { GetWindowThreadProcessId(hwnd, &mut process) };
            if process != 0 && process != std::process::id() {
                self.paste_target.set(hwnd);
            }
        }
    }

    fn icon_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd.get(),
            uID: 1,
            ..Default::default()
        }
    }

    fn add_icon(&self) -> bool {
        let mut data = self.icon_data();
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = TRAY_MESSAGE;
        data.hIcon = self.icon;
        let label = windows_sys::w!("剪贴板历史");
        // NOTIFYICONDATAW is packed on x86, so write the tooltip through a raw pointer.
        unsafe {
            ptr::copy_nonoverlapping(label, (&raw mut data.szTip).cast::<u16>(), 6);
        }
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) } != 0;
        self.has_icon.set(added);
        added
    }

    fn detach(&self) {
        let hwnd = self.hwnd.get();
        if hwnd.is_null() {
            return;
        }
        unsafe {
            if self.has_icon.replace(false) {
                Shell_NotifyIconW(NIM_DELETE, &self.icon_data());
            }
            if let Some((id, _)) = self.hotkey.take() {
                UnregisterHotKey(hwnd, id);
            }
            RemoveWindowSubclass(hwnd, Some(window_proc), 1);
        }
        self.hwnd.set(ptr::null_mut());
    }

    fn show(&self) {
        let hwnd = self.hwnd.get();
        unsafe {
            self.remember_paste_target(GetForegroundWindow());
            let command = if IsIconic(hwnd) != 0 {
                SW_SHOWNOACTIVATE
            } else {
                SW_SHOWNA
            };
            ShowWindow(hwnd, command);
            SetWindowPos(
                hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    fn menu(&self) {
        let hwnd = self.hwnd.get();
        let mut point = POINT::default();
        unsafe {
            if GetCursorPos(&mut point) == 0 {
                return;
            }
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            AppendMenuW(menu, MF_STRING, OPEN_MENU, windows_sys::w!("打开剪贴板"));
            AppendMenuW(menu, MF_STRING, EXIT_MENU, windows_sys::w!("退出"));
            let previous = GetForegroundWindow();
            SetForegroundWindow(hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                hwnd,
                ptr::null(),
            );
            DestroyMenu(menu);
            if !previous.is_null() && previous != hwnd && GetForegroundWindow() == hwnd {
                SetForegroundWindow(previous);
            }
            if self.hwnd.get().is_null() {
                return;
            }
            PostMessageW(hwnd, WM_NULL, 0, 0);
            match command as usize {
                OPEN_MENU => self.show(),
                EXIT_MENU => {
                    self.exiting.set(true);
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
                _ => {}
            }
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    state_ptr: usize,
) -> LRESULT {
    // The menu can dispatch nested messages and destroy the window.
    let state = unsafe {
        let state_ptr = state_ptr as *const WindowState;
        Rc::increment_strong_count(state_ptr);
        Rc::from_raw(state_ptr)
    };
    match message {
        WM_MOUSEACTIVATE if lparam as u16 == HTCLIENT as u16 => return MA_NOACTIVATE as LRESULT,
        WM_ACTIVATE if wparam as u16 != WA_INACTIVE as u16 => {
            state.remember_paste_target(lparam as HWND);
        }
        WM_SYSCOMMAND if wparam & 0xfff0 == SC_MINIMIZE as usize && state.has_icon.get() => {
            unsafe { ShowWindow(hwnd, SW_HIDE) };
            return 0;
        }
        WM_SIZE if wparam == SIZE_MINIMIZED as usize && state.has_icon.get() => {
            unsafe { PostMessageW(hwnd, HIDE_MINIMIZED, 0, 0) };
        }
        HIDE_MINIMIZED => {
            if state.has_icon.get() && unsafe { IsIconic(hwnd) } != 0 {
                unsafe { ShowWindow(hwnd, SW_HIDE) };
            }
            return 0;
        }
        WM_CLOSE if state.close_to_tray.get() && state.has_icon.get() && !state.exiting.get() => {
            unsafe { ShowWindow(hwnd, SW_HIDE) };
            return 0;
        }
        TRAY_MESSAGE => {
            match lparam as u32 {
                WM_LBUTTONUP => state.show(),
                WM_RBUTTONUP => state.menu(),
                _ => {}
            }
            return 0;
        }
        WM_HOTKEY
            if state
                .hotkey
                .get()
                .is_some_and(|(id, _)| id as usize == wparam) =>
        {
            state.show();
            return 0;
        }
        WM_NCDESTROY => state.detach(),
        message if message == state.taskbar_created && message != 0 => {
            if !state.add_icon() {
                state.show();
            }
            return 0;
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}
