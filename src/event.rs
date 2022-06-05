use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The kind of of event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginEventKind {
    /// An editor is closed
    FileEditorClosed,
}

/// An event that can be handled.
/// Note that not all events are sent by default, and that you must subscribe to them!
#[derive(Debug, Serialize, Deserialize)]
pub enum PluginEvent {
    /// A file-backed editor was closed
    FileEditorClosed { path: PathBuf },
}
