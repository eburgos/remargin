//! Canonical JSON shapes for mutating ops whose core fn returns `()`.
//!
//! Each helper takes the inputs the response would otherwise compose
//! inline and returns the single shape both CLI and MCP emit.

use serde_json::{Value, json};

/// Shape emitted after `ack` / unack. `remove == true` switches the
/// top-level key from `acknowledged` to `unacknowledged`.
#[must_use]
pub fn ack(ids: &[String], remove: bool) -> Value {
    let key = if remove {
        "unacknowledged"
    } else {
        "acknowledged"
    };
    json!({ key: ids })
}

/// Shape emitted after `batch`.
#[must_use]
pub fn batch(ids: &[String]) -> Value {
    json!({ "ids": ids })
}

/// Shape emitted after `comment` (create).
#[must_use]
pub fn comment_created(id: &str) -> Value {
    json!({ "id": id })
}

/// Shape emitted after `delete`.
#[must_use]
pub fn comments_deleted(ids: &[String]) -> Value {
    json!({ "deleted": ids })
}

/// Shape emitted after `edit`.
#[must_use]
pub fn comment_edited(id: &str) -> Value {
    json!({ "edited": id })
}

/// Shape emitted after `react`. `remove == true` flips `action` from
/// `"added"` to `"removed"`.
#[must_use]
pub fn react(emoji: &str, comment_id: &str, remove: bool) -> Value {
    let action = if remove { "removed" } else { "added" };
    json!({
        "action": action,
        "emoji": emoji,
        "comment_id": comment_id,
    })
}
