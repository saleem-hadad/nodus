//! The Windows key hook: low-level keyboard and mouse hooks on a thread
//! with a message loop, as libuiohook installs them.
//!
//! A low-level hook's procedure gets no context of its own, so the sink it
//! reports to is kept in a process-wide slot; one hook runs at a time.

use std::sync::{Mutex, PoisonError, mpsc};
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, PeekMessageW,
    PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_QUIT,
    WM_USER,
};

use super::{HookSink, RunningHook, win_key, win_mouse};

/// Where the hook procedures report, while a hook runs.
static SINK: Mutex<Option<HookSink>> = Mutex::new(None);

fn report(heard: Option<super::HookEvent>) {
    if let Some(heard) = heard
        && let Some(sink) = SINK.lock().unwrap_or_else(PoisonError::into_inner).clone()
    {
        sink(heard);
    }
}

unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: for HC_ACTION, lparam points at the event's KBDLLHOOKSTRUCT.
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let _ = std::panic::catch_unwind(|| report(win_key(wparam as u32, event.vkCode)));
    }
    // SAFETY: passing the event on, as every hook must.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let _ = std::panic::catch_unwind(|| report(win_mouse(wparam as u32)));
    }
    // SAFETY: passing the event on, as every hook must.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// The running hooks' thread.
pub(super) struct WinHook {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

pub(super) fn start(sink: HookSink) -> Result<WinHook, String> {
    *SINK.lock().unwrap_or_else(PoisonError::into_inner) = Some(sink);
    let (ready, started) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("nodus-key-hook".into())
        .spawn(move || run(&ready))
        .map_err(|err| err.to_string())?;
    match started.recv() {
        Ok(Ok(thread_id)) => Ok(WinHook {
            thread_id,
            thread: Some(thread),
        }),
        Ok(Err(err)) => {
            let _ = thread.join();
            *SINK.lock().unwrap_or_else(PoisonError::into_inner) = None;
            Err(err)
        }
        Err(_) => Err("the key hook's thread ended".into()),
    }
}

fn run(ready: &mpsc::Sender<Result<u32, String>>) {
    // SAFETY: plain Win32 calls on this thread; the hooks are removed before
    // it ends, and `message` is valid for writes.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        // Makes this thread's message queue, so a stop posted early isn't lost.
        PeekMessageW(&mut message, std::ptr::null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
        let module = GetModuleHandleW(std::ptr::null());
        let keys: HHOOK = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), module, 0);
        let clicks: HHOOK = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), module, 0);
        if keys.is_null() || clicks.is_null() {
            if !keys.is_null() {
                UnhookWindowsHookEx(keys);
            }
            if !clicks.is_null() {
                UnhookWindowsHookEx(clicks);
            }
            let _ = ready.send(Err("SetWindowsHookEx failed".into()));
            return;
        }
        let _ = ready.send(Ok(GetCurrentThreadId()));
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        UnhookWindowsHookEx(keys);
        UnhookWindowsHookEx(clicks);
    }
}

impl RunningHook for WinHook {
    fn stop(mut self: Box<Self>) -> Result<(), String> {
        // SAFETY: posting WM_QUIT to the hooks' thread, which has a queue.
        let posted = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) } != 0;
        if !posted {
            return Err("couldn't tell the key hook to stop".into());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| "the key hook's thread panicked".to_owned())?;
        }
        *SINK.lock().unwrap_or_else(PoisonError::into_inner) = None;
        Ok(())
    }
}
