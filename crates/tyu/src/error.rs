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

    #[error("manifest: {0}")]
    Manifest(String),

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

    #[error("project: {0}")]
    Project(String),

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

    #[error("platform: {0}")]
    Platform(String),

    #[error("debug escalation: {0}")]
    Debug(String),

    #[error("high-water: {0}")]
    Highwater(String),

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

#[cfg(test)]
mod tests {
    use super::TyuError;

    #[test]
    fn string_backed_variants_render_own_subsystem_prefix() {
        let cases = [
            (TyuError::Manifest("x".into()), "manifest: x"),
            (TyuError::Project("x".into()), "project: x"),
            (TyuError::Graph("x".into()), "module graph: x"),
            (TyuError::Build("x".into()), "build: x"),
            (TyuError::Deploy("x".into()), "deploy: x"),
            (TyuError::Cache("x".into()), "cache: x"),
            (TyuError::Key("x".into()), "key: x"),
            (TyuError::Runner("x".into()), "runner: x"),
            (TyuError::Platform("x".into()), "platform: x"),
            (TyuError::Debug("x".into()), "debug escalation: x"),
            (TyuError::Highwater("x".into()), "high-water: x"),
            (TyuError::Test("x".into()), "test: x"),
            (TyuError::Provision("x".into()), "provisioning: x"),
            (TyuError::Toolchain("x".into()), "toolchain: x"),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }
}
