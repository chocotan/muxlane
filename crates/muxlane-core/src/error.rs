use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("{context}: {source}")]
    IoContext {
        context: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("TOML parse: {0}")]
    TomlParse(#[from] toml::de::Error),
    #[error("TOML serialization: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("invalid regex: {0}")]
    Regex(#[from] regex::Error),
    #[error("frame too large: {size} > {max}")]
    FrameTooLarge { size: usize, max: usize },
    #[error("stream closed")]
    Eof,
    #[error("unknown region: {0}")]
    UnknownRegion(String),
    #[error("invalid frame: {0}")]
    InvalidFrame(String),
}

pub type Result<T> = std::result::Result<T, Error>;
