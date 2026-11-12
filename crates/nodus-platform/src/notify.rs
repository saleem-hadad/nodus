//! System notifications and the Dock badge, as Electron 44 shows them
//! (`shell/browser/notifications/mac`, `…/win/windows_toast_notification.cc`).
//!
//! What's said and when belongs to the status track (HOST-202…HOST-215);
//! this only shows, takes back and reports clicks:
//!
//! - **macOS:** `UNUserNotificationCenter`. On first use it sets a delegate
//!   (clicks; banners, list and sound even while Nodus is in front) and asks
//!   for alerts, sounds and badges. Each notification has a title, a subtitle
//!   when there is one, a body, the default sound and a thread id (the
//!   project's) so Notification Center gathers a project's notifications. It
//!   needs an app bundle: an unbundled build (tests, `cargo run`) shows
//!   nothing, and an unsigned one may be refused by macOS.
//! - **Windows:** a toast for AppUserModelID `dev.saleem.nodus` (the Start
//!   menu shortcut's, see [`crate::app::set_app_user_model_id`]): the title
//!   and body as two lines, its group the project's, its tag the
//!   notification's id. Windows toasts have no subtitle.
//! - **The Dock badge** is macOS's only.

use std::sync::Arc;

/// What a notification says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    /// macOS only; empty for none.
    pub subtitle: String,
    pub body: String,
    /// Gathers notifications: the thread id on macOS, the toast group on
    /// Windows. Empty for none.
    pub group: String,
}

/// Called with a notification's id when the user clicks it.
pub type OnClick = Arc<dyn Fn(&str) + Send + Sync>;

/// Shows notifications and reports their clicks.
pub struct Notifier {
    #[cfg_attr(
        not(any(target_os = "macos", windows)),
        expect(dead_code, reason = "nothing shows notifications elsewhere")
    )]
    on_click: OnClick,
    #[cfg(target_os = "macos")]
    mac: mac::Center,
    #[cfg(windows)]
    windows: windows::Toasts,
}

impl std::fmt::Debug for Notifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Notifier")
    }
}

impl Notifier {
    /// A notifier that calls `on_click` with the id of a notification the
    /// user clicks. Nothing is set up with the OS until the first `show`.
    pub fn new(on_click: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self {
            on_click: Arc::new(on_click),
            #[cfg(target_os = "macos")]
            mac: mac::Center::default(),
            #[cfg(windows)]
            windows: windows::Toasts::default(),
        }
    }

    /// Electron's `Notification.isSupported()`: on macOS, only in an app
    /// bundle; always on Windows; never elsewhere (HOST-210).
    pub fn is_supported(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            mac::in_bundle()
        }
        #[cfg(windows)]
        {
            true
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            false
        }
    }

    /// Shows `notification` and returns its id, for [`close`](Self::close)
    /// and the click. Where notifications aren't supported nothing shows, but
    /// an id still comes back.
    pub fn show(&self, notification: &Notification) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        if self.is_supported() {
            #[cfg(target_os = "macos")]
            self.mac.show(&id, notification, &self.on_click);
            #[cfg(windows)]
            self.windows.show(&id, notification, &self.on_click);
        }
        let _ = notification;
        id
    }

    /// Takes a notification back from Notification Center or the Action
    /// Center (Electron's `notification.close()`).
    pub fn close(&self, id: &str) {
        if self.is_supported() {
            #[cfg(target_os = "macos")]
            self.mac.close(id);
            #[cfg(windows)]
            self.windows.close(id);
        }
        let _ = id;
    }
}

/// The Dock badge for `count` sessions waiting: the number, or nothing for 0
/// (`app.dock.setBadge(count ? String(count) : '')`).
pub fn badge_label(count: usize) -> String {
    if count == 0 { String::new() } else { count.to_string() }
}

/// Sets the Dock badge's label (`app.dock.setBadge`); empty clears it. It's
/// set on the main thread, so it shows once the app's run loop runs. Nothing
/// happens off macOS, which has no Dock.
pub fn set_dock_badge(label: &str) {
    #[cfg(target_os = "macos")]
    mac::set_dock_badge(label.to_owned());
    #[cfg(not(target_os = "macos"))]
    let _ = label;
}

/// The XML of a Windows toast, as Electron writes it: one `<text>` line
/// each for the title and the body, or just the one that's there
/// (`[no message]` when neither is).
pub fn toast_xml(title: &str, body: &str) -> String {
    let mut lines = Vec::new();
    if title.is_empty() || body.is_empty() {
        let line = if title.is_empty() { body } else { title };
        lines.push(if line.is_empty() { "[no message]" } else { line });
    } else {
        lines.push(title);
        lines.push(body);
    }
    let texts: String = lines
        .iter()
        .map(|line| format!("<text>{}</text>", escape_xml(line)))
        .collect();
    format!(r#"<toast><visual><binding template="ToastGeneric">{texts}</binding></visual></toast>"#)
}

/// The toast group: the notification's, or Electron's default.
pub fn toast_group(group: &str) -> &str {
    if group.is_empty() { "Notifications" } else { group }
}

fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::OnceLock;

    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
    use objc2_foundation::{NSArray, NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationDefaultActionIdentifier,
        UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse, UNNotificationSound,
        UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };

    use super::{Notification, OnClick};

    /// `UNUserNotificationCenter` throws in a process without a bundle.
    pub(super) fn in_bundle() -> bool {
        NSBundle::mainBundle().bundleIdentifier().is_some()
    }

    pub(super) struct DelegateIvars {
        on_click: OnClick,
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and Delegate
        // doesn't implement Drop.
        #[unsafe(super(NSObject))]
        #[name = "NodusNotificationCenterDelegate"]
        #[ivars = DelegateIvars]
        pub(super) struct Delegate;

        // SAFETY: NSObjectProtocol has no safety requirements.
        unsafe impl NSObjectProtocol for Delegate {}

        // SAFETY: the methods match the protocol's signatures.
        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            /// Shown even while Nodus is in front, as Electron does.
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                completion.call((UNNotificationPresentationOptions::List
                    | UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::Sound,));
            }

            /// A click on the notification itself (not a dismissal).
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                completion: &DynBlock<dyn Fn()>,
            ) {
                // SAFETY: a framework constant, set once the framework is loaded.
                let default = unsafe { UNNotificationDefaultActionIdentifier };
                if response.actionIdentifier().isEqualToString(default) {
                    let id = response.notification().request().identifier().to_string();
                    (self.ivars().on_click)(&id);
                }
                completion.call(());
            }
        }
    );

    impl Delegate {
        fn new(on_click: OnClick) -> Retained<Self> {
            let this = Self::alloc().set_ivars(DelegateIvars { on_click });
            // SAFETY: NSObject's init, on an allocated instance with its ivars set.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// The delegate, kept alive: the center only holds it weakly.
    struct Kept {
        _delegate: Retained<Delegate>,
    }

    // SAFETY: the delegate's only state is a Send + Sync closure, and
    // retaining or releasing an Objective-C object is thread-safe.
    unsafe impl Send for Kept {}
    // SAFETY: as above; nothing mutates it after it's made.
    unsafe impl Sync for Kept {}

    #[derive(Default)]
    pub(super) struct Center {
        delegate: OnceLock<Kept>,
    }

    impl Center {
        /// Electron's `NotificationPresenterMac`: set the delegate and ask
        /// for alerts, sounds and badges, once.
        fn center(&self, on_click: &OnClick) -> Retained<UNUserNotificationCenter> {
            let center = UNUserNotificationCenter::currentNotificationCenter();
            self.delegate.get_or_init(|| {
                let delegate = Delegate::new(on_click.clone());
                center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                let answered = RcBlock::new(|_granted: Bool, _error: *mut NSError| {});
                center.requestAuthorizationWithOptions_completionHandler(
                    UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge,
                    &answered,
                );
                Kept { _delegate: delegate }
            });
            center
        }

        pub(super) fn show(&self, id: &str, notification: &Notification, on_click: &OnClick) {
            let center = self.center(on_click);
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&notification.title));
            content.setSubtitle(&NSString::from_str(&notification.subtitle));
            content.setBody(&NSString::from_str(&notification.body));
            if !notification.group.is_empty() {
                content.setThreadIdentifier(&NSString::from_str(&notification.group));
            }
            content.setSound(Some(&UNNotificationSound::defaultSound()));
            let request =
                UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(id), &content, None);
            center.addNotificationRequest_withCompletionHandler(&request, None);
        }

        pub(super) fn close(&self, id: &str) {
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let ids = NSArray::from_retained_slice(&[NSString::from_str(id)]);
            center.removeDeliveredNotificationsWithIdentifiers(&ids);
        }
    }

    pub(super) fn set_dock_badge(label: String) {
        dispatch2::DispatchQueue::main().exec_async(move || {
            let Some(mtm) = objc2::MainThreadMarker::new() else {
                return;
            };
            let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
            app.dockTile().setBadgeLabel(Some(&NSString::from_str(&label)));
        });
    }
}

#[cfg(windows)]
mod windows {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, PoisonError};

    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::HSTRING;
    use windows_sys::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

    use super::{Notification, OnClick, toast_group, toast_xml};
    use crate::app::APP_ID;

    /// The toasts shown and not yet taken back, by id, kept so their clicks
    /// still arrive (Electron keeps its notification objects too).
    #[derive(Default)]
    pub(super) struct Toasts {
        shown: Arc<Mutex<HashMap<String, (ToastNotification, String)>>>,
    }

    /// WinRT needs COM on the calling thread; the multithreaded apartment
    /// suits any thread (an STA thread keeps its own, which works as well).
    fn ensure_com() {
        // SAFETY: a no-op on a thread where COM is already set up.
        let _ = unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32) };
    }

    impl Toasts {
        pub(super) fn show(&self, id: &str, notification: &Notification, on_click: &OnClick) {
            ensure_com();
            let group = toast_group(&notification.group).to_owned();
            let shown = (|| -> windows::core::Result<ToastNotification> {
                let xml = XmlDocument::new()?;
                xml.LoadXml(&HSTRING::from(toast_xml(&notification.title, &notification.body)))?;
                let toast = ToastNotification::CreateToastNotification(&xml)?;
                toast.SetGroup(&HSTRING::from(group.as_str()))?;
                toast.SetTag(&HSTRING::from(id))?;
                let clicked_id = id.to_owned();
                let on_click = on_click.clone();
                let shown = Arc::clone(&self.shown);
                toast.Activated(&TypedEventHandler::new(move |_, _| {
                    shown.lock().unwrap_or_else(PoisonError::into_inner).remove(&clicked_id);
                    on_click(&clicked_id);
                    Ok(())
                }))?;
                ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&toast)?;
                Ok(toast)
            })();
            if let Ok(toast) = shown {
                self.shown
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(id.to_owned(), (toast, group));
            }
        }

        pub(super) fn close(&self, id: &str) {
            ensure_com();
            let removed = self.shown.lock().unwrap_or_else(PoisonError::into_inner).remove(id);
            let Some((_, group)) = removed else { return };
            if let Ok(history) = ToastNotificationManager::History() {
                let _ = history.RemoveGroupedTagWithId(
                    &HSTRING::from(id),
                    &HSTRING::from(group.as_str()),
                    &HSTRING::from(APP_ID),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn writes_the_toast_electron_writes() {
        assert_eq!(
            toast_xml("Fix the login bug", "Needs you"),
            r#"<toast><visual><binding template="ToastGeneric"><text>Fix the login bug</text><text>Needs you</text></binding></visual></toast>"#
        );
        assert_eq!(
            toast_xml("", "Finished"),
            r#"<toast><visual><binding template="ToastGeneric"><text>Finished</text></binding></visual></toast>"#
        );
        assert_eq!(
            toast_xml("", ""),
            r#"<toast><visual><binding template="ToastGeneric"><text>[no message]</text></binding></visual></toast>"#
        );
        assert_eq!(
            toast_xml(r#"<b> & "q" 'a'"#, "ok"),
            r#"<toast><visual><binding template="ToastGeneric"><text>&lt;b&gt; &amp; &quot;q&quot; &apos;a&apos;</text><text>ok</text></binding></visual></toast>"#
        );
        assert_eq!(toast_group(""), "Notifications");
        assert_eq!(toast_group("project-1"), "project-1");
    }

    #[test]
    fn badges_the_waiting_count() {
        assert_eq!(badge_label(0), "");
        assert_eq!(badge_label(3), "3");
        assert_eq!(badge_label(12), "12");
    }

    /// A test binary has no bundle, so nothing reaches Notification Center:
    /// the calls still answer, and nothing is clicked.
    #[test]
    fn shows_nothing_without_a_bundle() {
        let clicks = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = Arc::clone(&clicks);
        let notifier = Notifier::new(move |id| seen.lock().unwrap().push(id.to_owned()));
        if cfg!(target_os = "macos") {
            assert!(!notifier.is_supported());
            let id = notifier.show(&Notification {
                title: "New session".into(),
                subtitle: "example".into(),
                body: "Needs you".into(),
                group: "project-1".into(),
            });
            assert_eq!(id.len(), 36);
            assert_ne!(notifier.show(&Notification::default()), id);
            notifier.close(&id);
            set_dock_badge(&badge_label(2));
        }
        assert!(clicks.lock().unwrap().is_empty());
    }
}
