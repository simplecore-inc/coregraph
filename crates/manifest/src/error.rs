use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest file not found at {path}")]
    NotFound { path: PathBuf },

    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to parse TOML in {path}: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to parse XML in {path}: {msg}")]
    Xml { path: PathBuf, msg: String },

    #[error("unsupported manifest format at {path}")]
    Unsupported { path: PathBuf },
}

impl ManifestError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn not_found(path: impl Into<PathBuf>) -> Self {
        Self::NotFound { path: path.into() }
    }

    pub fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.into(),
            source,
        }
    }

    pub fn toml(path: impl Into<PathBuf>, source: toml::de::Error) -> Self {
        Self::Toml {
            path: path.into(),
            source,
        }
    }

    pub fn xml(path: impl Into<PathBuf>, msg: impl Into<String>) -> Self {
        Self::Xml {
            path: path.into(),
            msg: msg.into(),
        }
    }
}
