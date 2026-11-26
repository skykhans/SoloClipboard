use gpui::{Keystroke, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};
use std::{cell::Cell, io, mem, ptr, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetKeyState, RegisterHotKey, SendInput, UnregisterHotKey, VkKeyScanW,
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL,
            MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU, VK_RWIN,
            VK_SHIFT, VK_V,
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
const DISMISS_WINDOW: u32 = WM_APP + 103;
const ICON_RESOURCE: *const u16 = 101 as *const u16;
const OPEN_MENU: usize = 1;
const EXIT_MENU: usize = 2;
const STARTUP_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_VALUE: &str = "SoloClipboard";
const FILE_NOT_FOUND_HRESULT: i32 = 0x80070002_u32 as i32;
const VK_MENU_MASK: u16 = 0xe8;
const MODIFIER_KEYS: [u16; 5] = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN];
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
    pub const WIN_V: Self = Self {
        modifiers: MOD_WIN,
        key: VK_V as u32,
    };

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
            _ => {
                let mut characters = stroke.key.chars();
                let character = u16::try_from(u32::from(characters.next()?)).ok()?;
                if characters.next().is_some() {
                    return None;
                }
                let key = unsafe { VkKeyScanW(character) } as u8;
                if !key.is_ascii_digit() {
                    return None;
                }
                u32::from(key)
            }
        };
        // GPUI can fold Shift into the character for number-row keys.
        let shift = unsafe { GetKeyState(i32::from(VK_SHIFT)) } < 0;
        let modifiers = (u32::from(stroke.modifiers.control) * MOD_CONTROL)
            | (u32::from(stroke.modifiers.alt) * MOD_ALT)
            | (u32::from(shift) * MOD_SHIFT)
            | (u32::from(stroke.modifiers.platform) * MOD_WIN);
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
            shortcut: Some(Shortcut::WIN_V),
        }
    }
}

#[derive(Clone, Copy)]
struct RegisteredShortcut {
    id: i32,
    shortcut: Shortcut,
}

impl RegisteredShortcut {
    fn register(hwnd: HWND, id: i32, shortcut: Shortcut) -> io::Result<Self> {
        if shortcut == Shortcut::WIN_V {
            WIN_V_TARGET.set(Some(WinVTarget {
                hwnd,
                id,
                v_down: false,
            }));
        } else if unsafe {
            RegisterHotKey(hwnd, id, shortcut.modifiers | MOD_NOREPEAT, shortcut.key)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { id, shortcut })
    }

    fn unregister(self, hwnd: HWND) {
        if self.shortcut == Shortcut::WIN_V {
            WIN_V_TARGET.set(None);
        } else {
            unsafe { UnregisterHotKey(hwnd, self.id) };
        }
    }
}

#[derive(Clone, Copy)]
struct WinVTarget {
    hwnd: HWND,
    id: i32,
    v_down: bool,
}

thread_local! {
    static WIN_V_TARGET: Cell<Option<WinVTarget>> = const { Cell::new(None) };
    static INPUT_WINDOW: Cell<HWND> = const { Cell::new(ptr::null_mut()) };
    static ESCAPE_DOWN: Cell<bool> = const { Cell::new(false) };
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let key = unsafe { (*(lparam as *const KBDLLHOOKSTRUCT)).vkCode } as u16;
        let message = wparam as u32;
        if match key {
            VK_V => intercept_win_v(message),
            VK_ESCAPE => intercept_escape(message),
            _ => false,
        } {
            return 1;
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

fn intercept_win_v(message: u32) -> bool {
    WIN_V_TARGET.with(|target| {
        let Some(mut state) = target.get() else {
            return false;
        };
        match message {
            WM_KEYUP | WM_SYSKEYUP if state.v_down => {
                state.v_down = false;
                target.set(Some(state));
                true
            }
            WM_KEYDOWN | WM_SYSKEYDOWN if state.v_down => true,
            WM_KEYDOWN | WM_SYSKEYDOWN if win_v_modifiers_down() => {
                state.v_down = true;
                target.set(Some(state));
                // Prevent the Win release from opening Start after swallowing V.
                let mask = [
                    keyboard_input(VK_MENU_MASK, 0),
                    keyboard_input(VK_MENU_MASK, KEYEVENTF_KEYUP),
                ];
                unsafe {
                    SendInput(
                        mask.len() as u32,
                        mask.as_ptr(),
                        mem::size_of::<INPUT>() as i32,
                    );
                    PostMessageW(state.hwnd, WM_HOTKEY, state.id as usize, 0);
                }
                true
            }
            _ => false,
        }
    })
}

fn intercept_escape(message: u32) -> bool {
    match message {
        WM_KEYUP | WM_SYSKEYUP => ESCAPE_DOWN.replace(false),
        WM_KEYDOWN if ESCAPE_DOWN.get() => true,
        WM_KEYDOWN => {
            let hwnd = INPUT_WINDOW.get();
            unsafe {
                if IsWindowVisible(hwnd) == 0
                    || belongs_to_app(GetForegroundWindow())
                    || MODIFIER_KEYS.into_iter().any(key_is_down)
                {
                    return false;
                }
                let posted = PostMessageW(hwnd, DISMISS_WINDOW, 0, 0) != 0;
                ESCAPE_DOWN.set(posted);
                posted
            }
        }
        _ => false,
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && matches!(
            wparam as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    {
        let hwnd = INPUT_WINDOW.get();
        unsafe {
            let point = (*(lparam as *const MSLLHOOKSTRUCT)).pt;
            let foreground = GetForegroundWindow();
            if IsWindowVisible(hwnd) != 0
                && (foreground == hwnd || !belongs_to_app(foreground))
                && !belongs_to_app(WindowFromPoint(point))
            {
                PostMessageW(hwnd, DISMISS_WINDOW, 0, 0);
            }
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

fn belongs_to_app(hwnd: HWND) -> bool {
    let mut process = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut process) };
    process == std::process::id()
}

struct InputHooks {
    keyboard: HHOOK,
    mouse: HHOOK,
}

impl InputHooks {
    fn install(hwnd: HWND) -> io::Result<Self> {
        unsafe {
            let module = GetModuleHandleW(ptr::null());
            let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
            if keyboard.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
            if mouse.is_null() {
                let error = io::Error::last_os_error();
                UnhookWindowsHookEx(keyboard);
                return Err(error);
            }
            INPUT_WINDOW.set(hwnd);
            Ok(Self { keyboard, mouse })
        }
    }
}

impl Drop for InputHooks {
    fn drop(&mut self) {
        INPUT_WINDOW.set(ptr::null_mut());
        ESCAPE_DOWN.set(false);
        unsafe {
            UnhookWindowsHookEx(self.mouse);
            UnhookWindowsHookEx(self.keyboard);
        }
    }
}

fn key_is_down(key: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(key)) < 0 }
}

fn win_v_modifiers_down() -> bool {
    (key_is_down(VK_LWIN) || key_is_down(VK_RWIN))
        && ![VK_CONTROL, VK_MENU, VK_SHIFT].into_iter().any(key_is_down)
}

fn keyboard_input(key: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

pub fn window_hwnd(window: &Window) -> Result<HWND, raw_window_handle::HandleError> {
    let handle = HasWindowHandle::window_handle(window)?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return Err(raw_window_handle::HandleError::NotSupported);
    };
    Ok(handle.hwnd.get() as HWND)
}

pub struct TrayWindow(Rc<WindowState>);

struct WindowState {
    hwnd: Cell<HWND>,
    icon: HICON,
    taskbar_created: u32,
    has_icon: Cell<bool>,
    close_to_tray: Cell<bool>,
    exiting: Cell<bool>,
    hotkey: Cell<Option<RegisteredShortcut>>,
    input_hooks: Cell<Option<InputHooks>>,
    paste_target: Cell<HWND>,
}

impl TrayWindow {
    pub fn new(
        window: &Window,
        close_to_tray: bool,
        previous_window: HWND,
    ) -> Result<Self, String> {
        let hwnd = window_hwnd(window).map_err(|error| error.to_string())?;
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
                input_hooks: Cell::new(None),
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
            tray.0.input_hooks.set(Some(
                InputHooks::install(hwnd).map_err(|error| error.to_string())?,
            ));
            // Explorer reads the taskbar style when the window is shown.
            let visible = IsWindowVisible(hwnd) != 0;
            if visible {
                ShowWindow(hwnd, SW_HIDE);
            }
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                (style & !(WS_EX_APPWINDOW as isize)) | WS_EX_TOOLWINDOW as isize,
            );
            tray.set_interactive(false);
            if visible {
                ShowWindow(hwnd, SW_SHOWNA);
            }
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
        if MODIFIER_KEYS.into_iter().any(key_is_down) {
            return Err("请松开修饰键后再粘贴".into());
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
            .map(|(key, flags)| keyboard_input(key, flags));
            if SendInput(
                keys.len() as u32,
                keys.as_ptr(),
                mem::size_of::<INPUT>() as i32,
            ) != keys.len() as u32
            {
                return Err("无法向原来的输入窗口发送粘贴操作".into());
            }
        }
        self.0.hide();
        Ok(())
    }

    pub fn set_close_to_tray(&self, enabled: bool) {
        self.0.close_to_tray.set(enabled);
    }

    pub fn hide(&self) {
        self.0.hide();
    }

    pub fn set_interactive(&self, enabled: bool) {
        let hwnd = self.0.hwnd.get();
        let no_activate = WS_EX_NOACTIVATE as isize;
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let style = if enabled {
                style & !no_activate
            } else {
                style | no_activate
            };
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        }
    }

    pub fn set_shortcut(
        &self,
        shortcut: Option<Shortcut>,
        save: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let previous = self.0.hotkey.get();
        if previous.map(|registered| registered.shortcut) == shortcut {
            return save();
        }
        let hwnd = self.0.hwnd.get();
        let id = if previous.is_some_and(|registered| registered.id == 1) {
            2
        } else {
            1
        };
        let registered = shortcut
            .map(|shortcut| {
                RegisteredShortcut::register(hwnd, id, shortcut)
                    .map_err(|error| format!("无法注册快捷键 {}：{error}", shortcut.label()))
            })
            .transpose()?;
        if let Err(error) = save() {
            if let Some(registered) = registered {
                registered.unregister(hwnd);
            }
            return Err(error);
        }
        if let Some(previous) = previous {
            previous.unregister(hwnd);
        }
        self.0.hotkey.set(registered);
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
            if let Some(registered) = self.hotkey.take() {
                registered.unregister(hwnd);
            }
            drop(self.input_hooks.take());
            RemoveWindowSubclass(hwnd, Some(window_proc), 1);
        }
        self.hwnd.set(ptr::null_mut());
    }

    fn show(&self, z_order: HWND) {
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
                z_order,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    fn hide(&self) {
        let hwnd = self.hwnd.get();
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
            SetWindowPos(
                hwnd,
                HWND_NOTOPMOST,
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
                OPEN_MENU => self.show(HWND_TOPMOST),
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
        WM_MOUSEACTIVATE
            if lparam as u16 == HTCLIENT as u16
                && unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } & WS_EX_NOACTIVATE as isize
                    != 0 =>
        {
            return MA_NOACTIVATE as LRESULT;
        }
        WM_ACTIVATE if wparam as u16 != WA_INACTIVE as u16 => {
            state.remember_paste_target(lparam as HWND);
        }
        DISMISS_WINDOW => {
            state.hide();
            return 0;
        }
        WM_SYSCOMMAND if wparam & 0xfff0 == SC_MINIMIZE as usize && state.has_icon.get() => {
            state.hide();
            return 0;
        }
        WM_SIZE if wparam == SIZE_MINIMIZED as usize && state.has_icon.get() => {
            unsafe { PostMessageW(hwnd, HIDE_MINIMIZED, 0, 0) };
        }
        HIDE_MINIMIZED => {
            if state.has_icon.get() && unsafe { IsIconic(hwnd) } != 0 {
                state.hide();
            }
            return 0;
        }
        WM_CLOSE if state.close_to_tray.get() && state.has_icon.get() && !state.exiting.get() => {
            state.hide();
            return 0;
        }
        TRAY_MESSAGE => {
            match lparam as u32 {
                WM_LBUTTONUP => state.show(HWND_TOPMOST),
                WM_RBUTTONUP => state.menu(),
                _ => {}
            }
            return 0;
        }
        WM_HOTKEY
            if state
                .hotkey
                .get()
                .is_some_and(|registered| registered.id as usize == wparam) =>
        {
            state.show(HWND_TOPMOST);
            return 0;
        }
        WM_NCDESTROY => state.detach(),
        message if message == state.taskbar_created && message != 0 => {
            if !state.add_icon() {
                state.show(HWND_TOP);
            }
            return 0;
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}
