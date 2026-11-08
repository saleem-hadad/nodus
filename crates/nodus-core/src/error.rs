//! What a command that fails gives back, in the words 0.9.4 used.

/// A command that failed. It carries the IPC channel the command had in 0.9.4
/// and the JavaScript error's name, so the UI can show either form the
/// Electron app showed (PLAT-124):
///
/// - [`CommandError::message`], the bare message, where the UI stripped the
///   prefix (Settings, capture, views);
/// - [`CommandError::ipc_message`], `Error invoking remote method '<channel>':
///   Error: <message>`, where the UI showed the whole string (most toasts).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct CommandError {
    /// The 0.9.4 IPC channel, e.g. `projects:update`.
    pub channel: &'static str,
    /// The JavaScript error's name: `Error`, or `TypeError` for a type error.
    pub name: &'static str,
    /// The message, verbatim.
    pub message: String,
}

impl CommandError {
    /// An `Error` thrown by the command on `channel`.
    pub fn new(channel: &'static str, message: impl Into<String>) -> Self {
        Self {
            channel,
            name: "Error",
            message: message.into(),
        }
    }

    /// The bare message, as Settings and the capture panel showed it.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// What the UI's promise rejected with in 0.9.4:
    /// `Error invoking remote method '<channel>': <name>: <message>`.
    pub fn ipc_message(&self) -> String {
        format!(
            "Error invoking remote method '{}': {}: {}",
            self.channel, self.name, self.message
        )
    }
}

/// A command's result.
pub type CommandResult<T> = Result<T, CommandError>;

/// Gives an error from a domain crate the channel of the command it failed in.
pub(crate) trait OnChannel<T> {
    fn on(self, channel: &'static str) -> CommandResult<T>;
}

impl<T, E: std::fmt::Display> OnChannel<T> for Result<T, E> {
    fn on(self, channel: &'static str) -> CommandResult<T> {
        self.map_err(|err| CommandError::new(channel, err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_message_as_electron_wrapped_it() {
        let err = CommandError::new("projects:update", "Unknown project p1");
        assert_eq!(err.to_string(), "Unknown project p1");
        assert_eq!(
            err.ipc_message(),
            "Error invoking remote method 'projects:update': Error: Unknown project p1"
        );
    }
}
