use crate::paths::Paths;
use crate::tasks::{Emit, Task};
use std::sync::Arc;

/// Усе, що потрібно ядру для роботи: шляхи, HTTP-клієнт, канал подій у інтерфейс.
/// Не залежить від Tauri, тож ядро запускається й з інтеграційних тестів.
#[derive(Clone)]
pub struct Ctx {
    pub paths: Paths,
    pub http: reqwest::Client,
    pub emit: Emit,
    pub concurrency: usize,
}

impl Ctx {
    pub fn task(&self, title: impl Into<String>) -> Arc<Task> {
        Task::new(self.emit.clone(), title, None)
    }

    pub fn task_for(&self, title: impl Into<String>, tag: &str) -> Arc<Task> {
        Task::new(self.emit.clone(), title, Some(tag.to_string()))
    }

    pub fn emit(&self, event: &str, payload: serde_json::Value) {
        (self.emit)(event, payload)
    }
}
