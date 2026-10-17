// protocol/mod.rs - Re-exports domain modules for backward compatibility
// This module organizes the IPC protocol by domain, replacing the flat 2.3k-LOC
// protocol.rs with focused submodules while maintaining wire format compatibility.

pub mod clipboard;
pub mod launcher;
pub mod media;
pub mod shortcuts;
pub mod store;
pub mod windows;
pub mod doctor;
pub mod power;

// Common types used across domains
pub use crate::protocol::{ErrorKind, WindowOutcome};

// Facade enums that wrap domain-specific request/response types.
// This maintains the public API and wire format compatibility while organizing
// the implementation by concern.

#[derive(Debug, Clone)]
pub enum Request {
    // Core protocol
    Ping,
    Toggle,
    Show,
    Hide,
    Query { text: String },
    Doctor,
    Shutdown,
    AttachWindow,
    WindowOutcome(WindowOutcome),
    RecordLaunch { key: String },

    // Domain-specific requests
    Clipboard(clipboard::Request),
    Launcher(launcher::Request),
    Media(media::Request),
    Shortcuts(shortcuts::Request),
    Store(store::Request),
    Windows(windows::Request),
    Power(power::Request),
}

#[derive(Debug, Clone)]
pub enum Response {
    // Acks
    Ack,
    Pong,

    // Domain-specific responses
    Clipboard(clipboard::Response),
    Launcher(launcher::Response),
    Media(media::Response),
    Shortcuts(shortcuts::Response),
    Store(store::Response),
    Windows(windows::Response),
    Power(power::Response),

    // Window command (pushed only on attached connections)
    Window(WindowCommand),

    // Errors
    Error(Error),
}

// Represents a command pushed to the window connection.
#[derive(Debug, Clone)]
pub struct WindowCommand {
    // ... existing fields
}

// Represents a protocol error response.
#[derive(Debug, Clone)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}
