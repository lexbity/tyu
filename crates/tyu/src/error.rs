//! Unified error type for the `tyu` host tool.

/// Errors from `tyu` operations.
#[derive(Debug, thiserror::Error)]
pub enum TyuError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("parsing manifest '{}': {source}", path.display())]
    ManifestParse {
        path: std::path::PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("manifest read: {0}")]
    ManifestRead(std::io::Error),

    #[error("unknown profile '{0}'")]
    UnknownProfile(String),

    #[error("unknown feature '{0}'")]
    UnknownFeature(String),

    #[error("feature '{feature}' not supported by target '{target}'")]
    FeatureUnsupportedByTarget {
        feature: codegen_core::Feature,
        target: String,
    },

    #[error("project manifest parse: {0}")]
    ProjectParse(#[from] toml::de::Error),

    #[error("module graph: {0}")]
    Graph(String),

    #[error("build: {0}")]
    Build(String),

    #[error("compilation failed on '{}'", path.display())]
    CompileFailed { path: std::path::PathBuf },

    #[error(".o not produced at '{}'", path.display())]
    ObjectNotProduced { path: std::path::PathBuf },

    #[error("tool not found: {0}")]
    ToolNotFound(String),

    #[error("deploy: {0}")]
    Deploy(String),

    #[error("cache: {0}")]
    Cache(String),

    #[error("key: {0}")]
    Key(String),

    #[error("runner: {0}")]
    Runner(String),

    #[error("non-UTF-8 path: {0}")]
    NonUtf8Path(std::path::PathBuf),

    #[error("non-UTF-8 target triple")]
    NonUtf8Triple,

    #[error("test: {0}")]
    Test(String),

    #[error("provisioning: {0}")]
    Provision(String),

    #[error("toolchain: {0}")]
    Toolchain(String),
}

impl From<String> for TyuError {
    fn from(s: String) -> Self {
        TyuError::Build(s)
    }
}

impl From<&str> for TyuError {
    fn from(s: &str) -> Self {
        TyuError::Build(s.to_string())
    }
}

/// Allow `?` to convert `TyuError` to `String` for modules that still
/// use the legacy `Result<_, String>` return type.  This is a transitional
/// shim — new code should return `Result<_, TyuError>`.
impl From<TyuError> for String {
    fn from(e: TyuError) -> Self {
        e.to_string()
    }
}
