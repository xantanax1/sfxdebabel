use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Ошибка сети: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Ошибка базы данных: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Ошибка сериализации: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Ошибка декодирования аудио: {0}")]
    AudioDecode(String),

    #[error("Ошибка воспроизведения: {0}")]
    AudioPlayback(String),

    #[error("Ошибка экспорта WAV: {0}")]
    AudioExport(String),

    #[error("Ошибка источника звуков: {0}")]
    Provider(String),

    #[error("Ошибка ввода-вывода: {0}")]
    Io(#[from] std::io::Error),

    #[error("Некорректные параметры: {0}")]
    InvalidInput(String),

    #[error("Не задан API-ключ Freesound")]
    MissingApiKey,
}

impl From<AppError> for String {
    fn from(err: AppError) -> Self {
        err.to_string()
    }
}
