#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::ffi::CString;
use std::rc::Rc;
use std::sync::Mutex;

use lazy_static::lazy_static;
use std::os::raw::c_int;
use wchar::wch_c;
use winapi::ctypes;
use winapi::shared::guiddef;
use winapi::shared::minwindef;
use winapi::shared::windef;
use winapi::um::commctrl;
use winapi::um::errhandlingapi;
use winapi::um::libloaderapi;
use winapi::um::processthreadsapi;
use winapi::um::shellapi;
use winapi::um::stringapiset;
use winapi::um::wingdi;
use winapi::um::winnls;
use winapi::um::winuser;
use winapi::um::winver;

fn main() {
    let mut app = App::new();
    while app.poll() {}
}

struct App {
    notification: Notification,
    hwnd: windef::HWND,
}

impl App {
    pub fn new() -> Self {
        let hinstance = get_current_hinstance();

        let icon = Icon::new(hinstance, wch_c!("app_icon"));

        let hwnd = unsafe {
            commctrl::InitCommonControls();

            let template = wch_c!("default_dialog");
            let hwnd = winuser::CreateDialogParamW(
                hinstance,
                template.as_ptr(),
                std::ptr::null_mut(),
                Some(dialog_proc),
                0,
            );
            if hwnd.is_null() {
                let code = errhandlingapi::GetLastError();
                println!("Got error: {}", code);
            }

            hwnd
        };

        let title = wch_c!("topmost");

        let mut notification = Notification::new(hwnd, &icon, title);
        notification.show();

        Self { notification, hwnd }
    }

    pub fn poll(&mut self) -> bool {
        let mut msg = match get_message() {
            Some(msg) => msg,
            None => return false,
        };

        if unsafe { winuser::IsDialogMessageW(self.hwnd, &mut msg as winuser::LPMSG) } == 0 {
            return true;
        }

        unsafe {
            winuser::TranslateMessage(&msg as *const winuser::MSG);
            winuser::DispatchMessageW(&msg as *const winuser::MSG);
        }

        true
    }
}

fn get_message() -> Option<winuser::MSG> {
    let mut msg = winuser::MSG::default();
    match unsafe { winuser::GetMessageW(&mut msg as *mut winuser::MSG, std::ptr::null_mut(), 0, 0) }
    {
        0 => return None,
        _ => Some(msg),
    }
}

struct Notification {
    hwnd: windef::HWND,
    data: shellapi::NOTIFYICONDATAW,
    visible: bool,
}

impl Notification {
    pub fn new(hwnd: windef::HWND, icon: &Icon, title: &[u16]) -> Self {
        let mut info_title_buffer = [0u16; 128];
        info_title_buffer[0..title.len()].copy_from_slice(title);

        let data = shellapi::NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<shellapi::NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: shellapi::NIF_ICON | shellapi::NIF_MESSAGE | shellapi::NIF_TIP,
            uCallbackMessage: SWM_TRAY_MSG,
            hIcon: icon.data as windef::HICON,
            szTip: info_title_buffer,
            dwState: 0,
            dwStateMask: 0,
            szInfo: [0; 256],
            u: Default::default(),
            szInfoTitle: [0; 64],
            dwInfoFlags: 0,
            guidItem: guiddef::IID_NULL,
            hBalloonIcon: std::ptr::null_mut(),
        };

        Self {
            hwnd,
            data,
            visible: false,
        }
    }

    pub fn show(&mut self) {
        if !self.visible {
            unsafe {
                shellapi::Shell_NotifyIconW(
                    shellapi::NIM_ADD,
                    &mut self.data as shellapi::PNOTIFYICONDATAW,
                )
            };
            self.visible = true;
        }
    }

    pub fn hide(&mut self) {
        if self.visible {
            unsafe {
                shellapi::Shell_NotifyIconW(
                    shellapi::NIM_DELETE,
                    &mut self.data as shellapi::PNOTIFYICONDATAW,
                )
            };
        }
    }
}

impl Drop for Notification {
    fn drop(&mut self) {
        self.hide();
    }
}

unsafe extern "system" fn enum_window_callback(
    hwnd: windef::HWND,
    visible_windows: minwindef::LPARAM,
) -> minwindef::BOOL {
    let buffer_len = winuser::GetWindowTextLengthW(hwnd) + 1;

    let mut title = Vec::<u16>::new();
    title.resize(buffer_len as usize, 0);
    winapi::um::winuser::GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);

    if title.len() > 1
        && winuser::IsWindowVisible(hwnd) == minwindef::TRUE
        && winuser::IsWindow(hwnd) == minwindef::TRUE
    {
        let visible_windows = &mut *(visible_windows as *mut VisibleWindows);
        visible_windows.push(VisibleWindow { hwnd, title });
    }

    minwindef::TRUE
}

unsafe fn show_context_window(hwnd: windef::HWND) {
    let mut point = windef::POINT::default();
    winuser::GetCursorPos(&mut point as windef::LPPOINT);
    let hmenu = winuser::CreatePopupMenu();
    if hmenu.is_null() {
        return;
    }

    let mut visible_windows = VISIBLE_WINDOWS.lock().unwrap();
    visible_windows.clear();
    winuser::EnumWindows(
        Some(enum_window_callback),
        &mut *visible_windows as *mut _ as isize,
    );

    for (i, item) in visible_windows.iter().enumerate() {
        winuser::AppendMenuW(
            hmenu,
            winuser::MF_BYPOSITION | winuser::MF_UNCHECKED,
            SWM_TOGGLE_BEGIN as usize + i,
            item.title.as_ptr(),
        );
    }

    winuser::AppendMenuW(hmenu, winuser::MF_MENUBARBREAK, 0, std::ptr::null());

    let exit_label = wch_c!("Exit");
    winuser::InsertMenuW(
        hmenu,
        u32::MAX,
        winuser::MF_BYPOSITION,
        SWM_EXIT as usize,
        exit_label.as_ptr(),
    );

    winuser::SetForegroundWindow(hwnd);

    winuser::TrackPopupMenu(
        hmenu,
        winuser::TPM_BOTTOMALIGN,
        point.x,
        point.y,
        0,
        hwnd,
        std::ptr::null(),
    );
    winuser::DestroyMenu(hmenu);
}

unsafe extern "system" fn dialog_proc(
    hwnd: windef::HWND,
    message: minwindef::UINT,
    wparam: minwindef::WPARAM,
    lparam: minwindef::LPARAM,
) -> isize {
    match message {
        SWM_TRAY_MSG => match lparam as u32 {
            winuser::WM_RBUTTONDOWN | winuser::WM_CONTEXTMENU => {
                show_context_window(hwnd);
            }
            _ => {}
        },
        winuser::WM_COMMAND => {
            let wm_id = (wparam & 0xffff) as u32;
            let wm_event = ((wparam >> 16) & 0xffff) as u32;

            println!("EVENT: {}, LPARAM: {}, ID: {:x}", wm_event, lparam, wm_id);

            match wm_id {
                SWM_EXIT => {
                    winuser::DestroyWindow(hwnd);
                }
                id if id & SWM_TOGGLE_BEGIN == SWM_TOGGLE_BEGIN => {
                    let items = VISIBLE_WINDOWS.lock().unwrap();
                    if let Some(item) = items.get((id & !SWM_TOGGLE_BEGIN) as usize) {
                        println!("Open item");
                        set_foreground_window_internal(item.hwnd);
                    }
                }
                _ => {}
            }
            return 1;
        }
        winuser::WM_CLOSE => {
            winuser::DestroyWindow(hwnd);
        }
        winuser::WM_DESTROY => {
            println!("Destroy!");
            winuser::PostQuitMessage(0);
        }
        _ => return winuser::DefWindowProcW(hwnd, message, wparam, lparam),
    }

    0
}

struct Icon {
    data: windef::HICON,
}

impl Icon {
    pub fn new(hinstance: minwindef::HINSTANCE, name: &[u16]) -> Self {
        let icon = unsafe {
            winuser::LoadImageW(
                hinstance,
                name.as_ptr(),
                winuser::IMAGE_ICON,
                winuser::GetSystemMetrics(winuser::SM_CXSMICON),
                winuser::GetSystemMetrics(winuser::SM_CYSMICON),
                winuser::LR_DEFAULTCOLOR,
            )
        };
        Self {
            data: icon as windef::HICON,
        }
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe { winuser::DestroyIcon(self.data) };
    }
}

unsafe fn set_foreground_window_internal(hwnd: windef::HWND) {
    if winuser::IsWindow(hwnd) != minwindef::TRUE {
        return;
    }

    //relation time of SetForegroundWindow lock
    let mut lock_time_out = 0;
    let current_hwnd = winuser::GetForegroundWindow();
    let current_thread_id = processthreadsapi::GetCurrentThreadId();
    let window_thread_process_id =
        winuser::GetWindowThreadProcessId(current_hwnd, std::ptr::null_mut());

    if current_thread_id != window_thread_process_id {
        winuser::AttachThreadInput(current_thread_id, window_thread_process_id, minwindef::TRUE);

        winuser::SystemParametersInfoW(
            winuser::SPI_GETFOREGROUNDLOCKTIMEOUT,
            0,
            &mut lock_time_out as *mut c_int as *mut ctypes::c_void,
            0,
        );
        winuser::SystemParametersInfoW(
            winuser::SPI_SETFOREGROUNDLOCKTIMEOUT,
            0,
            std::ptr::null_mut(),
            winuser::SPIF_SENDWININICHANGE | winuser::SPIF_UPDATEINIFILE,
        );

        winuser::AllowSetForegroundWindow(winuser::ASFW_ANY);
    }

    if winuser::SetForegroundWindow(hwnd) != minwindef::TRUE {
        let code = errhandlingapi::GetLastError();
        println!("Failed to set foreground window: {}", code);
    }

    if current_thread_id != window_thread_process_id {
        winuser::SystemParametersInfoW(
            winuser::SPI_SETFOREGROUNDLOCKTIMEOUT,
            0,
            &mut lock_time_out as *mut c_int as *mut ctypes::c_void,
            winuser::SPIF_SENDWININICHANGE | winuser::SPIF_UPDATEINIFILE,
        );
        winuser::AttachThreadInput(
            current_thread_id,
            window_thread_process_id,
            minwindef::FALSE,
        );
    }
}

fn get_current_hwnd() -> windef::HWND {
    unsafe { winuser::GetActiveWindow() }
}

fn get_current_hinstance() -> minwindef::HINSTANCE {
    unsafe { libloaderapi::GetModuleHandleW(std::ptr::null()) }
}

struct VisibleWindow {
    hwnd: windef::HWND,
    title: Vec<u16>,
}

unsafe impl Send for VisibleWindow {}

type VisibleWindows = Vec<VisibleWindow>;

lazy_static! {
    static ref VISIBLE_WINDOWS: Mutex<VisibleWindows> = Mutex::new(Vec::new());
}

const TRAY_ICON_ID: u32 = 1;

const SWM_TRAY_MSG: u32 = winuser::WM_APP;
const SWM_EXIT: u32 = winuser::WM_APP + 1;
const SWM_TOGGLE_BEGIN: u32 = winuser::WM_APP | 0x4000;
