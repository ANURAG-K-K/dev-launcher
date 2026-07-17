//! Serializable backend error, mapped to `{ code, message }` at the IPC boundary
//! (architecture §9). Every fallible command returns `Result<T, AppError>`.

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("scan failed: {0}")]
    Scan(String),
    #[error("launch failed: {0}")]
    Launch(String),
    #[error("persistence error: {0}")]
    Persist(String),
    #[error("dependency cycle: {0}")]
    Cycle(String),
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
