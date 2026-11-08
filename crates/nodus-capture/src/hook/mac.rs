//! The macOS key hook: a CGEventTap on a thread of its own.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread::JoinHandle;

use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, kCFRunLoopCommonModes, kCFRunLoopDefaultMode};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy, CGEventType,
};

use super::{HookSink, RunningHook, mac_event, mac_types, system_fields};

/// What the tap's callback reads.
struct Tap {
    sink: HookSink,
    /// The tap itself, to switch it back on when macOS switches it off.
    port: OnceLock<CFRetained<CFMachPort>>,
}

/// The tap's run loop, which another thread stops.
struct Loop(CFRetained<CFRunLoop>);

// SAFETY: CFRunLoopStop and CFRunLoopWakeUp may be called from any thread;
// nothing else is done with it off its own thread.
unsafe impl Send for Loop {}

/// The running tap.
pub(super) struct MacHook {
    run_loop: Loop,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Starts the tap on its own thread, waiting until it's on or failed.
pub(super) fn start(sink: HookSink) -> Result<MacHook, String> {
    let (ready, started) = mpsc::channel();
    let stopping = Arc::new(AtomicBool::new(false));
    let stop = stopping.clone();
    let thread = std::thread::Builder::new()
        .name("nodus-key-hook".into())
        .spawn(move || run(sink, &stop, &ready))
        .map_err(|err| err.to_string())?;
    match started.recv() {
        Ok(Ok(run_loop)) => Ok(MacHook {
            run_loop,
            stopping,
            thread: Some(thread),
        }),
        Ok(Err(err)) => {
            let _ = thread.join();
            Err(err)
        }
        Err(_) => Err("the key hook's thread ended".into()),
    }
}

fn run(sink: HookSink, stopping: &AtomicBool, ready: &mpsc::Sender<Result<Loop, String>>) {
    let tap = Box::new(Tap {
        sink,
        port: OnceLock::new(),
    });
    let info: *const Tap = &*tap;
    // SAFETY: the callback reads `tap` through `info`; `tap` outlives the
    // port, which is invalidated before `tap` is dropped below.
    let port = unsafe {
        CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            // Active, as libuiohook's: Accessibility, not Input Monitoring.
            CGEventTapOptions::Default,
            mac_types::MASK,
            Some(callback),
            info.cast_mut().cast(),
        )
    };
    let Some(port) = port else {
        let _ = ready.send(Err("the event tap couldn't be created".into()));
        return;
    };
    let _ = tap.port.set(port.clone());
    let Some(source) = CFMachPort::new_run_loop_source(None, Some(&port), 0) else {
        port.invalidate();
        let _ = ready.send(Err("the event tap has no run loop source".into()));
        return;
    };
    let Some(run_loop) = CFRunLoop::current() else {
        port.invalidate();
        let _ = ready.send(Err("no run loop for the event tap".into()));
        return;
    };
    // SAFETY: a framework constant, set once CoreFoundation is loaded.
    run_loop.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
    CGEvent::tap_enable(&port, true);
    if ready.send(Ok(Loop(run_loop.clone()))).is_err() {
        stopping.store(true, Ordering::SeqCst);
    }
    // A stop asked before the loop runs isn't lost: the flag is read each turn.
    while !stopping.load(Ordering::SeqCst) {
        // SAFETY: a framework constant, set once CoreFoundation is loaded.
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 1.0, false);
    }
    CGEvent::tap_enable(&port, false);
    port.invalidate();
    drop(source);
    drop(tap);
}

/// The tap's callback: every event goes on unchanged; what it means goes to
/// the sink.
unsafe extern "C-unwind" fn callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: NonNull<CGEvent>,
    info: *mut c_void,
) -> *mut CGEvent {
    // SAFETY: `info` is the `Tap` that `run` keeps alive while the tap is.
    let tap = unsafe { &*info.cast::<Tap>() };
    // SAFETY: macOS hands the callback a valid event for its duration.
    let event_ref = unsafe { event.as_ref() };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if matches!(
            event_type.0,
            mac_types::TAP_DISABLED_BY_TIMEOUT | mac_types::TAP_DISABLED_BY_USER_INPUT
        ) {
            // Switched off for being slow, or for secure input: back on.
            if let Some(port) = tap.port.get() {
                CGEvent::tap_enable(port, true);
            }
            return;
        }
        let keycode = CGEvent::integer_value_field(Some(event_ref), CGEventField::KeyboardEventKeycode);
        let flags = CGEvent::flags(Some(event_ref)).0;
        let system = (event_type.0 == mac_types::SYSTEM_DEFINED)
            .then(|| CGEvent::new_data(None, Some(event_ref)))
            .flatten()
            .and_then(|data| system_fields(&data.to_vec()));
        if let Some(heard) = mac_event(event_type.0, keycode, flags, system) {
            (tap.sink)(heard);
        }
    }));
    event.as_ptr()
}

impl RunningHook for MacHook {
    fn stop(mut self: Box<Self>) -> Result<(), String> {
        self.stopping.store(true, Ordering::SeqCst);
        self.run_loop.0.stop();
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| "the key hook's thread panicked".to_owned())?;
        }
        Ok(())
    }
}
