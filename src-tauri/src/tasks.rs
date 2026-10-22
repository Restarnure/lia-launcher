use crate::error::{Error, Result};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Як ядро повідомляє інтерфейс (у застосунку — події Tauri, у тестах — друк або нічого).
pub type Emit = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static CANCEL_FLAGS: LazyLock<Mutex<HashMap<u64, Arc<AtomicBool>>>> = LazyLock::new(Default::default);

pub fn cancel(id: u64) {
    if let Some(f) = CANCEL_FLAGS.lock().unwrap().get(&id) {
        f.store(true, Ordering::Relaxed);
    }
}

/// Довга операція з прогресом (завантаження, встановлення). Подія `task` у фронтенд.
pub struct Task {
    pub id: u64,
    title: String,
    /// Чого стосується задача (id збірки) — щоб інтерфейс показав прогрес на потрібній кнопці.
    tag: Option<String>,
    emit: Emit,
    cancel: Arc<AtomicBool>,
    done: AtomicU64,
    total: AtomicU64,
    stage: Mutex<(String, &'static str)>,
    last: Mutex<Instant>,
}

impl Task {
    pub fn new(emit: Emit, title: impl Into<String>, tag: Option<String>) -> Arc<Task> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        CANCEL_FLAGS.lock().unwrap().insert(id, cancel.clone());
        let t = Arc::new(Task {
            id,
            title: title.into(),
            tag,
            emit,
            cancel,
            done: AtomicU64::new(0),
            total: AtomicU64::new(0),
            stage: Mutex::new(("prepare".into(), "")),
            last: Mutex::new(Instant::now() - Duration::from_secs(1)),
        });
        t.send("running", None);
        t
    }

    /// Новий етап: `stage` — ключ для перекладу у фронтенді, `unit` — "bytes" | "items" | "".
    pub fn stage(&self, stage: &str, total: u64, unit: &'static str) {
        *self.stage.lock().unwrap() = (stage.to_string(), unit);
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        self.send("running", None);
    }

    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }

    pub fn add(&self, delta: u64) {
        self.done.fetch_add(delta, Ordering::Relaxed);
        self.throttled();
    }

    /// Відкат прогресу невдалої спроби завантаження (щоб повтор не рахувався двічі).
    pub fn sub(&self, delta: u64) {
        let _ = self.done.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |d| Some(d.saturating_sub(delta)));
    }

    pub fn set(&self, done: u64) {
        self.done.store(done, Ordering::Relaxed);
        self.throttled();
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn check(&self) -> Result<()> {
        if self.cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn finish<T>(&self, r: &Result<T>) {
        match r {
            Ok(_) => self.send("done", None),
            Err(Error::Cancelled) => self.send("cancelled", None),
            Err(e) => self.send("error", Some(e)),
        }
        CANCEL_FLAGS.lock().unwrap().remove(&self.id);
    }

    fn throttled(&self) {
        let mut last = self.last.lock().unwrap();
        if last.elapsed() >= Duration::from_millis(120) {
            *last = Instant::now();
            drop(last);
            self.send("running", None);
        }
    }

    fn send(&self, state: &str, error: Option<&Error>) {
        let (stage, unit) = self.stage.lock().unwrap().clone();
        (self.emit)(
            "task",
            json!({
                "id": self.id,
                "title": self.title,
                "tag": self.tag,
                "stage": stage,
                "unit": unit,
                "done": self.done.load(Ordering::Relaxed),
                "total": self.total.load(Ordering::Relaxed),
                "state": state,
                "error": error,
            }),
        );
    }
}
