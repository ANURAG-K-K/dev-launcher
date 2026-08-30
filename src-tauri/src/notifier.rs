//! Notifier (F15, architecture §3.6) - optional Windows desktop notifications for
//! launch-completed, repo-crashed, build-failed, and restart-completed events. Off by default,
//! gated by the `notifications_enabled` setting.
//!
//! TODO: add `tauri-plugin-notification`, subscribe to process status transitions, and emit toasts.
