//! Bounded no-follow reads and durable publications inside owned release-control directories.

use super::{Error, Result, require};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path},
};

pub(super) fn io(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.to_owned(),
        source,
    }
}

pub(super) fn absolute(path: &Path) -> Result<()> {
    require(
        path.is_absolute()
            && !path
                .as_os_str()
                .to_string_lossy()
                .split('/')
                .any(|part| part == "." || part == ".."),
        format!(
            "expected absolute path without traversal: {}",
            path.display()
        ),
    )?;
    let mut current = std::path::PathBuf::from("/");
    for part in path.components() {
        if let Component::Normal(part) = part {
            current.push(part);
            match fs::symlink_metadata(&current) {
                Ok(metadata) => require(
                    !metadata.file_type().is_symlink(),
                    format!("symbolic link in release path: {}", current.display()),
                )?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io(&current, error)),
            }
        }
    }
    Ok(())
}

pub(super) fn directory(path: &Path) -> Result<Metadata> {
    absolute(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| io(path, error))?;
    require(
        metadata.is_dir(),
        format!("expected release directory: {}", path.display()),
    )?;
    Ok(metadata)
}

pub(super) fn regular(path: &Path) -> Result<Metadata> {
    absolute(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| io(path, error))?;
    require(
        metadata.is_file(),
        format!("expected regular release file: {}", path.display()),
    )?;
    Ok(metadata)
}

pub(super) fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let before = regular(path)?;
    require(
        before.len() <= limit,
        format!(
            "release file exceeds {limit}-byte limit: {}",
            path.display()
        ),
    )?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| io(path, error))?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io(path, error))?;
    let after = file.metadata().map_err(|error| io(path, error))?;
    let current = regular(path)?;
    require(
        bytes.len() as u64 <= limit && same(&before, &after) && same(&after, &current),
        format!("release file changed while reading: {}", path.display()),
    )?;
    Ok(bytes)
}

pub(super) fn digest_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn digest(path: &Path) -> Result<String> {
    let before = regular(path)?;
    require(
        before.len() <= 1024 * 1024 * 1024,
        "release file exceeds single-file limit",
    )?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| io(path, error))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    let mut total = 0_u64;
    loop {
        let count = file.read(&mut buffer).map_err(|error| io(path, error))?;
        if count == 0 {
            break;
        }
        total += count as u64;
        require(
            total <= 1024 * 1024 * 1024,
            "release payload grew beyond byte limit",
        )?;
        hash.update(&buffer[..count]);
    }
    let after = file.metadata().map_err(|error| io(path, error))?;
    require(
        same(&before, &after) && same(&after, &regular(path)?),
        "release payload changed while hashing",
    )?;
    Ok(format!("{:x}", hash.finalize()))
}

fn same(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mode() == right.mode()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

pub(super) fn owned(path: &Path) -> Result<Metadata> {
    let metadata = directory(path)?;
    require(
        metadata.uid() == nix::unistd::geteuid().as_raw() && metadata.mode() & 0o022 == 0,
        format!(
            "release control directory must be owned and not group/world writable: {}",
            path.display()
        ),
    )?;
    Ok(metadata)
}

pub(super) fn tree(path: &Path, mode: u32) -> Result<()> {
    absolute(path)?;
    match fs::symlink_metadata(path) {
        Ok(_) => {
            owned(path)?;
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io(path, error)),
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::Invalid("release directory has no parent".into()))?;
    tree(parent, mode)?;
    fs::DirBuilder::new()
        .mode(mode)
        .create(path)
        .map_err(|error| io(path, error))?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| io(path, error))?;
    sync(path)?;
    sync(parent)
}

pub(super) fn sync(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| io(path, error))
}

pub(super) fn control_file(path: &Path) -> Result<()> {
    let metadata = regular(path)?;
    require(
        metadata.uid() == nix::unistd::geteuid().as_raw()
            && metadata.nlink() == 1
            && metadata.mode() & 0o022 == 0
            && metadata.mode() & 0o7000 == 0,
        format!("unsafe release control file: {}", path.display()),
    )
}

pub(super) fn publish(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Invalid("publication has no parent".into()))?;
    owned(parent)?;
    match fs::symlink_metadata(path) {
        Ok(_) => control_file(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io(path, error)),
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| io(parent, error))?;
    temporary
        .write_all(bytes)
        .map_err(|error| io(path, error))?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|error| io(path, error))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| io(path, error))?;
    temporary
        .persist(path)
        .map_err(|error| io(path, error.error))?;
    sync(parent)
}

pub(super) fn json(path: &Path, value: &impl Serialize) -> Result<()> {
    publish(path, &serde_json::to_vec_pretty(value)?, 0o600)
}

pub(super) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    use serde::{
        Deserialize, Deserializer,
        de::{Error as _, MapAccess, SeqAccess, Visitor},
    };
    struct Unique(serde_json::Value);
    impl<'de> Deserialize<'de> for Unique {
        fn deserialize<D: Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
            de.deserialize_any(UniqueVisitor).map(Self)
        }
    }
    struct UniqueVisitor;
    impl<'de> Visitor<'de> for UniqueVisitor {
        type Value = serde_json::Value;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("JSON without duplicate keys")
        }
        fn visit_map<A: MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut out = serde_json::Map::new();
            while let Some((key, value)) = map.next_entry::<String, Unique>()? {
                if out.insert(key.clone(), value.0).is_some() {
                    return Err(A::Error::custom(format!(
                        "duplicate release JSON key: {key}"
                    )));
                }
            }
            Ok(serde_json::Value::Object(out))
        }
        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(value) = seq.next_element::<Unique>()? {
                out.push(value.0);
            }
            Ok(serde_json::Value::Array(out))
        }
        fn visit_bool<E: serde::de::Error>(
            self,
            value: bool,
        ) -> std::result::Result<Self::Value, E> {
            Ok(value.into())
        }
        fn visit_u64<E: serde::de::Error>(self, value: u64) -> std::result::Result<Self::Value, E> {
            Ok(value.into())
        }
        fn visit_i64<E: serde::de::Error>(self, value: i64) -> std::result::Result<Self::Value, E> {
            Ok(value.into())
        }
        fn visit_f64<E: serde::de::Error>(self, value: f64) -> std::result::Result<Self::Value, E> {
            serde_json::Number::from_f64(value)
                .map(serde_json::Value::Number)
                .ok_or_else(|| E::custom("nonfinite JSON number"))
        }
        fn visit_str<E: serde::de::Error>(
            self,
            value: &str,
        ) -> std::result::Result<Self::Value, E> {
            Ok(value.into())
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(serde_json::Value::Null)
        }
    }
    let unique: Unique = serde_json::from_slice(bytes)?;
    Ok(serde_json::from_value(unique.0)?)
}
