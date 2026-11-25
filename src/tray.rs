use std::{io, mem::size_of, ptr::{null, null_mut}};
use tokio::sync::watch;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};

const CALLBACK: u32 = WM_APP + 1;
const NOTIFY_READY: u32 = WM_APP + 2;
const OPEN: usize = 1;
const EXIT: usize = 2;

fn wide(text: &str) -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() }

struct State { url: Vec<u16>, stop: watch::Sender<bool>, taskbar_created: u32 }

unsafe fn icon(window: HWND, action: u32) -> bool {
    unsafe {
        let mut data: NOTIFYICONDATAW = std::mem::zeroed();
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = window;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = CALLBACK;
        data.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
        let tip = wide("67ai");
        data.szTip[..tip.len()].copy_from_slice(&tip);
        Shell_NotifyIconW(action, &data) != 0
    }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *const State;
        if !state.is_null() {
            let state = &*state;
            if message == state.taskbar_created { icon(window, NIM_ADD); return 0; }
            match message {
                NOTIFY_READY => {
                    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
                    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
                    data.hWnd = window;
                    data.uID = 1;
                    data.uFlags = NIF_INFO;
                    data.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;
                    let title = wide("67ai: ready for approval");
                    let text = wide(&format!("All tasks are done and merged. {w} task(s) waiting for approval. Click to open the board."));
                    data.szInfoTitle[..title.len()].copy_from_slice(&title);
                    data.szInfo[..text.len()].copy_from_slice(&text);
                    let shown = Shell_NotifyIconW(NIM_MODIFY, &data);
                    if shown == 0 { eprintln!("Could not show the 67ai notification"); }
                    return shown as LRESULT;
                }
                CALLBACK if l as u32 == NIN_BALLOONUSERCLICK => {
                    SendMessageW(window, WM_COMMAND, OPEN, 0);
                    return 0;
                }
                CALLBACK if l as u32 == WM_RBUTTONUP => {
                    let menu = CreatePopupMenu();
                    AppendMenuW(menu, MF_STRING, OPEN, wide("Open in browser").as_ptr());
                    AppendMenuW(menu, MF_STRING, EXIT, wide("Exit").as_ptr());
                    let mut point = std::mem::zeroed();
                    GetCursorPos(&mut point);
                    SetForegroundWindow(window);
                    let command = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, point.x, point.y, 0, window, null());
                    DestroyMenu(menu);
                    PostMessageW(window, WM_NULL, 0, 0);
                    if command != 0 { SendMessageW(window, WM_COMMAND, command as usize, 0); }
                    return 0;
                }
                WM_COMMAND if w == OPEN => {
                    ShellExecuteW(window, wide("open").as_ptr(), state.url.as_ptr(), null(), null(), SW_SHOWNORMAL);
                    return 0;
                }
                WM_COMMAND if w == EXIT => { DestroyWindow(window); return 0; }
                WM_CLOSE => { DestroyWindow(window); return 0; }
                WM_DESTROY => {
                    icon(window, NIM_DELETE);
                    let _ = state.stop.send(true);
                    PostQuitMessage(0);
                    return 0;
                }
                _ => {}
            }
        }
        DefWindowProcW(window, message, w, l)
    }
}

pub struct Tray { window: usize, thread: Option<std::thread::JoinHandle<()>> }
impl Tray {
    pub fn notify_ready(&self, count: usize) {
        unsafe { PostMessageW(self.window as HWND, NOTIFY_READY, count, 0); }
    }

    pub fn start(port: u16, stop: watch::Sender<bool>) -> io::Result<Self> {
        let (ready, receive) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || unsafe {
            let class_name = wide("67aiTray");
            let instance = GetModuleHandleW(null());
            let class = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance, lpszClassName: class_name.as_ptr(), ..std::mem::zeroed() };
            if RegisterClassW(&class) == 0 { let _ = ready.send(Err(io::Error::last_os_error())); return; }
            let window = CreateWindowExW(0, class_name.as_ptr(), class_name.as_ptr(), 0, 0, 0, 0, 0, null_mut(), null_mut(), instance, null());
            if window.is_null() { let _ = ready.send(Err(io::Error::last_os_error())); return; }
            let state = State { url: wide(&format!("http://127.0.0.1:{port}")), stop, taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
            SetWindowLongPtrW(window, GWLP_USERDATA, &state as *const State as isize);
            if !icon(window, NIM_ADD) {
                let error = io::Error::other("Could not create the 67ai tray icon");
                DestroyWindow(window);
                let _ = ready.send(Err(error));
                return;
            }
            let _ = ready.send(Ok(window as usize));
            let mut message = std::mem::zeroed();
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        });
        let window = receive.recv().map_err(io::Error::other)??;
        Ok(Self { window, thread: Some(thread) })
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        unsafe { PostMessageW(self.window as HWND, WM_CLOSE, 0, 0); }
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_exit_requests_shutdown() {
        let (stop, shutdown) = watch::channel(false);
        let tray = Tray::start(3038, stop).unwrap();
        assert!(!*shutdown.borrow());
        assert_ne!(unsafe { SendMessageW(tray.window as HWND, NOTIFY_READY, 2, 0) }, 0);
        unsafe { SendMessageW(tray.window as HWND, WM_COMMAND, EXIT, 0); }
        assert!(*shutdown.borrow());
        drop(tray);
    }
}
