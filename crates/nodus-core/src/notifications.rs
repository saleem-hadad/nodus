//! Showing notifications and the Dock badge: the OS's (nodus-platform) for
//! the app, none for a headless Nodus, or any [`Notifier`] a caller brings.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use nodus_sessions::notify::{Notification, NotificationId, Notifier};

use crate::nodus::{Inner, lock};

/// How the core tells the user about sessions while the app is in the
/// background (02, HOST-202…215).
#[derive(Clone, Default)]
pub enum Notifications {
    /// None at all, and no Dock badge: a headless Nodus, and tests.
    #[default]
    Off,
    /// The system's: Notification Center or toasts, and the Dock badge on
    /// macOS. A click opens the card it's about.
    System,
    /// Another way to show them. A click is passed back with
    /// [`Nodus::notification_clicked`](crate::Nodus::notification_clicked).
    Custom(Arc<dyn Notifier>),
}

/// Shows nothing; nothing is supported.
pub(crate) struct NoNotifier;

impl Notifier for NoNotifier {
    fn is_supported(&self) -> bool {
        false
    }
    fn show(&self, _: NotificationId, _: &Notification) {}
    fn close(&self, _: NotificationId) {}
    fn has_dock(&self) -> bool {
        false
    }
    fn set_badge(&self, _: &str) {}
}

/// The system's notifications (`nodus_platform::notify`), and the Dock badge.
pub(crate) struct SystemNotifier {
    platform: nodus_platform::notify::Notifier,
    /// Each notification shown, by our id, with the system's.
    shown: Arc<Mutex<HashMap<NotificationId, String>>>,
}

impl SystemNotifier {
    /// Clicks go to the core's notifications, while it's there.
    pub(crate) fn new(core: Weak<Inner>) -> Self {
        let shown: Arc<Mutex<HashMap<NotificationId, String>>> = Arc::default();
        let ids = Arc::clone(&shown);
        let platform = nodus_platform::notify::Notifier::new(move |clicked: &str| {
            let id = lock(&ids)
                .iter()
                .find(|(_, system)| system.as_str() == clicked)
                .map(|(id, _)| *id);
            if let (Some(id), Some(core)) = (id, core.upgrade()) {
                core.notify.clicked(id);
            }
        });
        Self { platform, shown }
    }
}

impl Notifier for SystemNotifier {
    fn is_supported(&self) -> bool {
        self.platform.is_supported()
    }

    fn show(&self, id: NotificationId, notification: &Notification) {
        let system = self.platform.show(&nodus_platform::notify::Notification {
            title: notification.title.clone(),
            subtitle: notification.subtitle.clone(),
            body: notification.body.clone(),
            group: notification.group_id.clone(),
        });
        lock(&self.shown).insert(id, system);
    }

    fn close(&self, id: NotificationId) {
        if let Some(system) = lock(&self.shown).remove(&id) {
            self.platform.close(&system);
        }
    }

    /// Electron's `app.dock`: macOS only.
    fn has_dock(&self) -> bool {
        cfg!(target_os = "macos")
    }

    fn set_badge(&self, text: &str) {
        nodus_platform::notify::set_dock_badge(text);
    }
}
