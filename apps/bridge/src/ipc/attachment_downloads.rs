use std::{
    collections::VecDeque,
    io::{self, Write},
    sync::Mutex,
};

use agent_room_bridge_ipc::IpcOpenedAttachment;
use tempfile::NamedTempFile;

const CACHE_LIMIT: usize = 128 * 1024 * 1024;

#[derive(Default)]
pub(super) struct AttachmentDownloads {
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<(NamedTempFile, usize)>,
    bytes: usize,
}

impl AttachmentDownloads {
    pub(super) fn save(
        &self,
        name: String,
        media_type: &str,
        bytes: &[u8],
    ) -> io::Result<IpcOpenedAttachment> {
        if bytes.len() > CACHE_LIMIT {
            return Err(io::Error::other("attachment.cache_limit"));
        }
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| io::Error::other("attachment.cache_unavailable"))?;
        while cache.bytes + bytes.len() > CACHE_LIMIT || cache.entries.len() >= 64 {
            if let Some((file, size)) = cache.entries.pop_front() {
                cache.bytes -= size;
                file.close()?;
            } else {
                break;
            }
        }
        // Remote names are display metadata only, never a filesystem path or executable extension.
        let suffix = match media_type {
            "image/png" => ".png",
            "image/jpeg" => ".jpg",
            "image/gif" => ".gif",
            "image/webp" => ".webp",
            "image/avif" => ".avif",
            "application/pdf" => ".pdf",
            "text/plain" | "text/markdown" => ".txt",
            _ => ".bin",
        };
        let mut file = tempfile::Builder::new()
            .prefix("agent-room-attachment-")
            .suffix(suffix)
            .tempfile()?;
        file.write_all(bytes)?;
        file.flush()?;
        let local_path = file
            .path()
            .to_str()
            .ok_or_else(|| io::Error::other("attachment.path_invalid"))?
            .to_owned();
        let attachment = IpcOpenedAttachment {
            name,
            local_path,
            byte_length: bytes.len() as u64,
        };
        cache.bytes += bytes.len();
        cache.entries.push_back((file, bytes.len()));
        Ok(attachment)
    }
}

#[cfg(test)]
mod tests {
    use super::AttachmentDownloads;

    #[test]
    fn 附件以原始字节保存且远端名称不能控制路径() {
        let downloads = AttachmentDownloads::default();
        let bytes = [0, 0xff, 0x80, 3];
        let attachment = downloads
            .save("../../remote.exe".into(), "image/png", &bytes)
            .expect("附件保存");
        assert_eq!(
            std::fs::read(&attachment.local_path).expect("可读取文件"),
            bytes
        );
        assert_eq!(
            std::path::Path::new(&attachment.local_path)
                .extension()
                .and_then(|ext| ext.to_str()),
            Some("png")
        );
        assert!(!attachment.local_path.contains("remote.exe"));
        assert_eq!(attachment.byte_length, 4);
        drop(downloads);
        assert!(!std::path::Path::new(&attachment.local_path).exists());
    }

    #[test]
    fn 重复读取有界且旧附件可清理() {
        let downloads = AttachmentDownloads::default();
        let first = downloads
            .save("first.bin".into(), "application/octet-stream", &[1])
            .expect("首个附件");
        for _ in 0..64 {
            downloads
                .save("other.bin".into(), "application/octet-stream", &[2])
                .expect("后续附件");
        }
        assert!(!std::path::Path::new(&first.local_path).exists());
    }
}
