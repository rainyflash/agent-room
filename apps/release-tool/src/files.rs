use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::Path,
};

use serde::Serialize;
use zeroize::Zeroizing;

use crate::error::{ToolError, ToolResult};

pub fn read_text(path: &Path) -> ToolResult<String> {
    fs::read_to_string(path).map_err(|source| ToolError::Io {
        path: path.to_path_buf(),
        source,
    })
}

pub fn write_new_json<T: Serialize>(path: &Path, value: &T) -> ToolResult<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    write_new_bytes(path, &bytes, false)
}

pub fn write_new_private_json<T: Serialize>(path: &Path, value: &T) -> ToolResult<()> {
    let bytes = Zeroizing::new(serde_json::to_vec_pretty(value)?);
    write_new_bytes(path, bytes.as_slice(), true)
}

fn write_new_bytes(path: &Path, bytes: &[u8], private: bool) -> ToolResult<()> {
    if path.exists() {
        return Err(ToolError::RefuseOverwrite(path.to_path_buf()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ToolError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_permissions(&mut options, private);
    let mut file = options.open(path).map_err(|source| ToolError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.write_all(bytes).map_err(|source| ToolError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.sync_all().map_err(|source| ToolError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(unix)]
fn configure_permissions(options: &mut OpenOptions, private: bool) {
    use std::os::unix::fs::OpenOptionsExt as _;

    options.mode(if private { 0o600 } else { 0o644 });
}

#[cfg(not(unix))]
fn configure_permissions(_options: &mut OpenOptions, _private: bool) {}
