use std::{
    collections::VecDeque,
    io::{self, Write},
    path::PathBuf,
    sync::Mutex,
};

use agent_room_bridge_ipc::IpcOpenedAttachment;
use tempfile::TempPath;

const CACHE_LIMIT: usize = 128 * 1024 * 1024;

#[derive(Default)]
pub(super) struct AttachmentDownloads {
    /// 未指定时退回进程临时目录；生产运行时固定使用 Bridge 数据根下的私有附件目录，
    /// 受限宿主正是按该目录被授予读取权限。
    directory: Option<PathBuf>,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    /// 只留路径、不留句柄：淘汰或 Bridge 退出时删掉文件。
    entries: VecDeque<(TempPath, usize)>,
    bytes: usize,
}

impl AttachmentDownloads {
    pub(super) fn new(directory: PathBuf) -> Self {
        Self {
            directory: Some(directory),
            cache: Mutex::default(),
        }
    }

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
            if let Some((path, size)) = cache.entries.pop_front() {
                cache.bytes -= size;
                path.close()?;
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
        let mut builder = tempfile::Builder::new();
        builder.prefix("agent-room-attachment-").suffix(suffix);
        let mut file = match self.directory.as_ref() {
            Some(directory) => {
                std::fs::create_dir_all(directory)?;
                builder.tempfile_in(directory)?
            }
            None => builder.tempfile()?,
        };
        file.write_all(bytes)?;
        file.flush()?;
        // 写完就关掉句柄。Windows 上开着可写句柄时，只许别人一起读的打开方式（比如 .NET 的
        // File.ReadAllText）会因为共享冲突失败，宿主就读不到附件（Alpha 67 实机验收）。
        let path = file.into_temp_path();
        let local_path = path
            .to_str()
            .ok_or_else(|| io::Error::other("attachment.path_invalid"))?
            .to_owned();
        let attachment = IpcOpenedAttachment {
            name,
            local_path,
            byte_length: bytes.len() as u64,
        };
        cache.bytes += bytes.len();
        cache.entries.push_back((path, bytes.len()));
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

    #[cfg(windows)]
    #[test]
    fn 附件写完就关掉句柄_只许别人一起读的打开方式也读得到() {
        use std::{io::Read, os::windows::fs::OpenOptionsExt};
        // .NET 的 File.ReadAllText 这样打开：只读，只许别人一起读。别人开着可写句柄就打不开。
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        let root = tempfile::tempdir().expect("临时目录");
        let downloads = AttachmentDownloads::new(root.path().join("attachments"));
        let attachment = downloads
            .save("note.txt".into(), "text/plain", b"hello")
            .expect("附件保存");
        let mut text = String::new();
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&attachment.local_path)
            .expect("Bridge 不再开着附件的可写句柄")
            .read_to_string(&mut text)
            .expect("可读取文件");
        assert_eq!(text, "hello");
        drop(downloads);
        assert!(!std::path::Path::new(&attachment.local_path).exists());
    }

    #[test]
    fn 指定目录时附件只写入该私有目录() {
        let root = tempfile::tempdir().expect("临时目录");
        let directory = root.path().join("attachments");
        let downloads = AttachmentDownloads::new(directory.clone());
        let attachment = downloads
            .save("note.txt".into(), "text/plain", b"hello")
            .expect("附件保存");
        assert!(std::path::Path::new(&attachment.local_path).starts_with(&directory));
        assert_eq!(
            std::fs::read(&attachment.local_path).expect("可读取文件"),
            b"hello"
        );
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
