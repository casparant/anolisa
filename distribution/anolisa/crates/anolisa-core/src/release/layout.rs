//! Versioned program directories are separate from XDG/FHS mutable application roots.

use super::{Result, files, model::Manifest, require};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Explicit release-install scope; effective UID never chooses it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Per-user architecture-specific release and XDG application roots.
    #[default]
    User,
    /// Root-managed local programs and resources, with separate administrator configuration.
    System,
}

impl Scope {
    /// AW CLI spelling for this scope.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::System => "system",
        }
    }
}

/// Process inputs captured once rather than changing global environment during installation.
#[derive(Clone, Debug)]
pub struct Environment {
    /// Absolute real home directory for user installation and stable launchers.
    pub home: PathBuf,
    /// Resolved absolute persistent-state root, before the application suffix.
    pub state: PathBuf,
    /// Resolved absolute configuration root, before the application suffix.
    pub config: PathBuf,
    /// Resolved absolute portable-data root, before the application suffix.
    pub data: PathBuf,
    /// Resolved absolute disposable-cache root, before the application suffix.
    pub cache: PathBuf,
    /// Optional session-manager runtime directory; install never invents one.
    pub runtime: Option<PathBuf>,
}

impl Environment {
    /// Resolve valid absolute XDG roots, ignoring empty or relative overrides.
    ///
    /// # Errors
    /// Rejects missing or unsafe HOME and symbolic-link traversal in resolved roots.
    pub fn detect() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| super::Error::Invalid("HOME is required for release layout".into()))?;
        Self::from_values(home, |name| std::env::var_os(name).map(PathBuf::from))
    }

    /// Resolve caller-supplied environment values without mutating the process environment.
    ///
    /// # Errors
    /// Rejects unsafe resolved paths.
    pub fn from_values(home: PathBuf, value: impl Fn(&str) -> Option<PathBuf>) -> Result<Self> {
        files::absolute(&home)?;
        let root = |name, fallback: &str| {
            value(name)
                .filter(|path| valid_root(path))
                .unwrap_or_else(|| home.join(fallback))
        };
        let result = Self {
            state: root("XDG_STATE_HOME", ".local/state"),
            config: root("XDG_CONFIG_HOME", ".config"),
            data: root("XDG_DATA_HOME", ".local/share"),
            cache: root("XDG_CACHE_HOME", ".cache"),
            runtime: value("XDG_RUNTIME_DIR")
                .filter(|path| valid_root(path))
                .filter(|path| {
                    use std::os::unix::fs::MetadataExt;
                    files::directory(path).is_ok_and(|metadata| {
                        metadata.uid() == nix::unistd::geteuid().as_raw()
                            && metadata.mode() & 0o777 == 0o700
                    })
                }),
            home,
        };
        for path in [&result.state, &result.config, &result.data, &result.cache] {
            files::absolute(path)?;
        }
        Ok(result)
    }
}

fn valid_root(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .as_os_str()
            .to_string_lossy()
            .split('/')
            .any(|part| part == "." || part == "..")
}

/// One selected target layout; the AW database deliberately remains outside the release.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReleaseLayout {
    /// Explicit privilege boundary.
    pub scope: Scope,
    /// ABI namespace, independent of portable XDG data.
    pub arch_id: String,
    /// Parent containing releases and the current reference.
    pub base: PathBuf,
    /// Final real prefix passed to AW, never renamed after registration.
    pub prefix: PathBuf,
    /// Real versioned destination of logical `share/` payloads, outside the program tree.
    pub resources: PathBuf,
    /// Public launcher directory outside all version prefixes.
    pub bin: PathBuf,
    /// Product operation records and lock directory.
    pub state: PathBuf,
    /// Application configuration outside the release.
    pub config: PathBuf,
    /// Scope default for mutable data; the system value requires explicit service configuration.
    pub data: PathBuf,
    /// Disposable artifact cache outside the release.
    pub cache: PathBuf,
    /// User session runtime or explicit system-service default; never binds Provider templates.
    pub runtime: Option<PathBuf>,
    /// AW package authority; system always remains /var/lib/aw/packages.
    pub aw_database: PathBuf,
    /// Normalized XDG state root supplied explicitly to AW subprocesses.
    pub xdg_state: PathBuf,
    /// Home-relative architecture library root used by shared-home launchers.
    pub user_library: PathBuf,
}

impl ReleaseLayout {
    /// Select new-release paths without using legacy FsLayout's whole-root rebasing.
    ///
    /// # Errors
    /// Rejects unsafe path components and overlapping state/payload authority.
    pub fn resolve(manifest: &Manifest, scope: Scope, environment: &Environment) -> Result<Self> {
        super::model::identifier(&manifest.release_id)?;
        let arch_id = format!("{}-linux-gnu", manifest.arch);
        let user_library = environment.home.join(".local/lib");
        let (base, bin, state, config, data, cache, runtime, aw_database) = match scope {
            Scope::User => (
                user_library.join(&arch_id).join("anolisa"),
                environment.home.join(".local/bin"),
                environment.state.join("anolisa"),
                environment.config.join("anolisa"),
                environment.data.join("anolisa"),
                environment.cache.join("anolisa"),
                environment
                    .runtime
                    .as_ref()
                    .map(|root| root.join("anolisa")),
                environment.state.join("aw/packages"),
            ),
            Scope::System => (
                PathBuf::from("/usr/local/lib/anolisa"),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/var/lib/anolisa"),
                PathBuf::from("/usr/local/etc/anolisa"),
                PathBuf::from("/var/lib/anolisa/data"),
                PathBuf::from("/var/cache/anolisa"),
                Some(PathBuf::from("/run/anolisa")),
                PathBuf::from("/var/lib/aw/packages"),
            ),
        };
        let prefix = base.join("releases").join(&manifest.release_id);
        let resources = match scope {
            Scope::User => environment
                .data
                .join("anolisa/resources/releases")
                .join(&arch_id)
                .join(&manifest.release_id),
            Scope::System => {
                PathBuf::from("/usr/local/share/anolisa/releases").join(&manifest.release_id)
            }
        };
        let layout = Self {
            scope,
            arch_id,
            base,
            prefix,
            resources,
            bin,
            state,
            config,
            data,
            cache,
            runtime,
            aw_database,
            xdg_state: environment.state.clone(),
            user_library,
        };
        for path in [
            &layout.base,
            &layout.prefix,
            &layout.resources,
            &layout.bin,
            &layout.state,
            &layout.config,
            &layout.data,
            &layout.cache,
            &layout.aw_database,
        ] {
            require(
                valid_root(path),
                format!("unsafe release layout path: {}", path.display()),
            )?;
        }
        require(
            !layout.resources.starts_with(&layout.prefix)
                && !layout.prefix.starts_with(&layout.resources),
            "release program and resources directories must be separate trees",
        )?;
        for path in [
            &layout.state,
            &layout.config,
            &layout.data,
            &layout.cache,
            &layout.aw_database,
        ] {
            require(
                !path.starts_with(&layout.prefix) && !layout.prefix.starts_with(path),
                "release prefix overlaps mutable application or AW state",
            )?;
        }
        for path in [
            &layout.state,
            &layout.config,
            &layout.cache,
            &layout.aw_database,
        ] {
            require(
                !path.starts_with(&layout.resources) && !layout.resources.starts_with(path),
                "release resources overlap configuration, state, cache or AW authority",
            )?;
        }
        Ok(layout)
    }

    /// Product-owned compatibility alias; AW owns files through the separate real resources root.
    pub fn resource_alias(&self) -> PathBuf {
        self.prefix.join("share")
    }

    /// Durable record for this release and architecture.
    pub fn record(&self, release_id: &str) -> PathBuf {
        self.state
            .join("releases")
            .join(&self.arch_id)
            .join(format!("{release_id}.json"))
    }

    /// Current-version pointer, owned by ANOLISA rather than AW.
    pub fn current(&self) -> PathBuf {
        self.base.join("current")
    }
}
