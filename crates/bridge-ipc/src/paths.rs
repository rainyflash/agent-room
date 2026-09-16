use std::path::{Path, PathBuf};

/// Verified attachment downloads share one private directory under the Bridge data root.
///
/// A confined reception host inspects attachments with its own read-only file tool, so the Bridge
/// and the receiver must agree on one directory: the receiver grants the host read access to
/// exactly this path and nothing else. Keeping it under the Bridge data root also keeps it
/// owner-only, unlike a predictable name inside a shared system temporary directory.
#[must_use]
pub fn attachment_directory(data_root: &Path) -> PathBuf {
    data_root.join("attachments")
}

#[cfg(test)]
mod tests {
    use super::attachment_directory;
    use std::path::Path;

    #[test]
    fn 附件目录固定位于数据根之下() {
        let data_root = Path::new("agent-room-data");
        let directory = attachment_directory(data_root);
        assert!(directory.starts_with(data_root));
        assert_eq!(
            directory.file_name().and_then(std::ffi::OsStr::to_str),
            Some("attachments")
        );
    }
}
