use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::wire::IpcFrame;

const MAX_FRAME_BYTES: usize = 64 * 1_024;

#[derive(Debug, Default)]
pub struct IpcFrameCodec;

impl IpcFrameCodec {
    /// 从长度前缀流读取单个闭合 JSON 帧。
    ///
    /// # Errors
    ///
    /// I/O 中断、空帧、超限帧或畸形 JSON 返回稳定协议错误。
    pub async fn read<R>(reader: &mut R) -> IpcProtocolResult<IpcFrame>
    where
        R: AsyncRead + Unpin,
    {
        let length = reader
            .read_u32()
            .await
            .map_err(|_| failure(IpcProtocolFailureKind::Io))?;
        let length =
            usize::try_from(length).map_err(|_| failure(IpcProtocolFailureKind::FrameTooLarge))?;
        if length == 0 {
            return Err(failure(IpcProtocolFailureKind::InvalidFrame));
        }
        if length > MAX_FRAME_BYTES {
            return Err(failure(IpcProtocolFailureKind::FrameTooLarge));
        }
        let mut bytes = vec![0; length];
        reader
            .read_exact(&mut bytes)
            .await
            .map_err(|_| failure(IpcProtocolFailureKind::Io))?;
        serde_json::from_slice(&bytes).map_err(|_| failure(IpcProtocolFailureKind::InvalidFrame))
    }

    /// 向长度前缀流写入单个闭合 JSON 帧。
    ///
    /// # Errors
    ///
    /// 序列化超限或 I/O 中断时返回稳定协议错误。
    pub async fn write<W>(writer: &mut W, frame: &IpcFrame) -> IpcProtocolResult<()>
    where
        W: AsyncWrite + Unpin,
    {
        let bytes =
            serde_json::to_vec(frame).map_err(|_| failure(IpcProtocolFailureKind::InvalidFrame))?;
        if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
            return Err(failure(IpcProtocolFailureKind::FrameTooLarge));
        }
        let length = u32::try_from(bytes.len())
            .map_err(|_| failure(IpcProtocolFailureKind::FrameTooLarge))?;
        writer
            .write_u32(length)
            .await
            .map_err(|_| failure(IpcProtocolFailureKind::Io))?;
        writer
            .write_all(&bytes)
            .await
            .map_err(|_| failure(IpcProtocolFailureKind::Io))?;
        writer
            .flush()
            .await
            .map_err(|_| failure(IpcProtocolFailureKind::Io))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcProtocolFailureKind {
    Io,
    InvalidFrame,
    FrameTooLarge,
    InvalidHandshake,
    AuthenticationRejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcProtocolFailure {
    kind: IpcProtocolFailureKind,
}

impl IpcProtocolFailure {
    pub const fn new(kind: IpcProtocolFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> IpcProtocolFailureKind {
        self.kind
    }
}

pub type IpcProtocolResult<T> = Result<T, IpcProtocolFailure>;

const fn failure(kind: IpcProtocolFailureKind) -> IpcProtocolFailure {
    IpcProtocolFailure::new(kind)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use tokio::io::{AsyncWriteExt as _, duplex};
    use uuid::Uuid;

    use crate::{IpcCaller, IpcFrame, IpcMethod, IpcScopeName, IpcVersion};

    use super::{IpcFrameCodec, IpcProtocolFailureKind, MAX_FRAME_BYTES};

    #[tokio::test]
    async fn 帧编解码保持闭合协议结构() {
        let (mut client, mut server) = duplex(4 * 1_024);
        let frame = IpcFrame::ClientHello {
            installation_id: "install_1".to_owned(),
            caller: IpcCaller::McpServer,
            supported_versions: vec![IpcVersion { major: 1, minor: 0 }],
            requested_scopes: vec![IpcScopeName::BridgeStatusRead],
        };

        IpcFrameCodec::write(&mut client, &frame)
            .await
            .expect("测试帧可写入");

        assert_eq!(
            IpcFrameCodec::read(&mut server)
                .await
                .expect("测试帧可读取"),
            frame
        );
    }

    #[tokio::test]
    async fn 未知字段与超限长度必须在分派前拒绝() {
        let (mut client, mut server) = duplex(4 * 1_024);
        let bytes = br#"{"type":"request","correlationId":"00000000-0000-0000-0000-000000000001","method":"bridge_status","unexpected":true}"#;
        client
            .write_u32(u32::try_from(bytes.len()).expect("测试长度有效"))
            .await
            .expect("测试长度可写入");
        client.write_all(bytes).await.expect("测试正文可写入");

        let failure = IpcFrameCodec::read(&mut server)
            .await
            .expect_err("未知字段必须失败");

        assert_eq!(failure.kind(), IpcProtocolFailureKind::InvalidFrame);

        let (mut client, mut server) = duplex(16);
        client
            .write_u32(64 * 1_024 + 1)
            .await
            .expect("测试长度可写入");
        assert_eq!(
            IpcFrameCodec::read(&mut server)
                .await
                .expect_err("超限帧必须失败")
                .kind(),
            IpcProtocolFailureKind::FrameTooLarge
        );
    }

    #[test]
    fn 请求方法不是任意字符串() {
        let frame = IpcFrame::Request {
            correlation_id: Uuid::from_u128(1),
            method: IpcMethod::BridgeStatus,
        };
        let encoded = serde_json::to_value(frame).expect("测试帧可序列化");

        assert_eq!(encoded["method"], "bridge_status");
    }

    proptest! {
        #[test]
        fn 任意有界字节帧只能被接受或稳定拒绝(payload in prop::collection::vec(any::<u8>(), 0..4_096)) {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("属性测试运行时可创建");
            let result = runtime.block_on(async {
                let (mut client, mut server) = duplex(payload.len().saturating_add(4).max(4));
                client
                    .write_u32(u32::try_from(payload.len()).expect("测试载荷长度可编码"))
                    .await
                    .expect("长度前缀可写入");
                client.write_all(&payload).await.expect("测试载荷可写入");
                drop(client);
                IpcFrameCodec::read(&mut server).await
            });

            let allowed = match result {
                Ok(_) => true,
                Err(failure) => matches!(
                    failure.kind(),
                    IpcProtocolFailureKind::InvalidFrame | IpcProtocolFailureKind::Io
                ),
            };
            prop_assert!(allowed);
        }

        #[test]
        fn 任意超限长度在分配正文前拒绝(length in (u32::try_from(MAX_FRAME_BYTES).expect("帧上限可编码") + 1)..=u32::MAX) {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("属性测试运行时可创建");
            let failure = runtime.block_on(async {
                let (mut client, mut server) = duplex(4);
                client.write_u32(length).await.expect("长度前缀可写入");
                IpcFrameCodec::read(&mut server).await.expect_err("超限帧必须失败")
            });

            prop_assert_eq!(failure.kind(), IpcProtocolFailureKind::FrameTooLarge);
        }
    }
}
