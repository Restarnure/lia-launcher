use serde::{Serialize, Serializer};

/// Помилка, яку бачить інтерфейс: `code` перекладається на фронтенді, `message` — технічна деталь.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{message}")]
    Coded { code: &'static str, message: String },
    #[error("{0}")]
    Net(#[from] reqwest::Error),
    #[error("{}", io_text(.0))]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("скасовано")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Текст системної помилки українською. Інакше Windows підставить мову ОС
/// (на багатьох ПК в Україні — російську).
fn io_text(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    let t = match e.kind() {
        NotFound => "файл або тека не існує",
        PermissionDenied => "немає доступу (файл зайнятий іншою програмою або заборонено)",
        AlreadyExists => "файл уже існує",
        StorageFull => "закінчилося місце на диску",
        ResourceBusy => "файл зайнятий іншою програмою",
        TimedOut => "час очікування вичерпано",
        ConnectionRefused | ConnectionReset | ConnectionAborted | NotConnected => "з'єднання розірвано",
        InvalidData | UnexpectedEof => "пошкоджені дані",
        _ => return e.to_string(),
    };
    match e.raw_os_error() {
        Some(code) => format!("{t} (код {code})"),
        None => t.to_string(),
    }
}

pub fn err(code: &'static str, message: impl Into<String>) -> Error {
    Error::Coded { code, message: message.into() }
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Error::Coded { code, .. } => code,
            Error::Net(_) => "net",
            Error::Io(_) => "io",
            Error::Json(_) => "json",
            Error::Zip(_) => "zip",
            Error::Cancelled => "cancelled",
        }
    }
}

/// Додає до помилки пояснення, що саме робили (шлях, URL).
pub trait Context<T> {
    fn ctx(self, what: impl FnOnce() -> String) -> Result<T>;
}

impl<T, E: Into<Error>> Context<T> for std::result::Result<T, E> {
    fn ctx(self, what: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|e| {
            let e: Error = e.into();
            match e {
                Error::Cancelled => Error::Cancelled,
                Error::Coded { code, message } => Error::Coded { code, message: format!("{}: {message}", what()) },
                other => Error::Coded { code: other.code(), message: format!("{}: {other}", what()) },
            }
        })
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("Error", 2)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}
