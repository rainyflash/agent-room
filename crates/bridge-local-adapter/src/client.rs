use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use agent_room_bridge_core::ipc::IpcCallerKind;
use agent_room_bridge_ipc::{
    IpcClientFailure, IpcClientFailureKind, IpcClientSession, IpcErrorCategory, IpcMethod,
    IpcResponse,
};
use interprocess::local_socket::tokio::{Stream, prelude::*};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::timeout,
};

use crate::{
    IpcCredentialFailure, IpcCredentialFailureKind, IpcCredentialSource, LocalIpcEndpoint,
    OsIpcCredentialReader, SecureStorageService,
};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(15);
// 关闭会话需排空长轮询、令牌刷新和持久化，再释放独立身份；普通请求的 15 秒不足以完成。
const CLOSE_SESSION_TIMEOUT: Duration = Duration::from_mins(2);

#[derive(Clone)]
pub struct LocalBridgeClient {
    runtime_root: PathBuf,
    credentials: Arc<dyn IpcCredentialSource>,
    caller: IpcCallerKind,
    connect_timeout: Duration,
    operation_timeout: Duration,
}

impl LocalBridgeClient {
    pub fn agent_cli(runtime_root: PathBuf, service: SecureStorageService) -> Self {
        Self::for_caller(runtime_root, IpcCallerKind::AgentCli, service)
    }
    /// 创建只能以通用 MCP Server 身份协商工具作用域的本地客户端。
    pub fn system(runtime_root: PathBuf) -> Self {
        Self::system_with_secure_storage_service(runtime_root, SecureStorageService::default())
    }

    /// 使用显式安全存储命名空间创建通用 MCP Server 客户端。
    pub fn system_with_secure_storage_service(
        runtime_root: PathBuf,
        service: SecureStorageService,
    ) -> Self {
        Self::for_caller(runtime_root, IpcCallerKind::McpServer, service)
    }

    /// 创建供受信桌面壳执行用户确认操作的本地客户端。
    pub fn desktop_shell(runtime_root: PathBuf) -> Self {
        Self::desktop_shell_with_secure_storage_service(
            runtime_root,
            SecureStorageService::default(),
        )
    }

    /// 使用显式安全存储命名空间创建受信桌面壳客户端。
    pub fn desktop_shell_with_secure_storage_service(
        runtime_root: PathBuf,
        service: SecureStorageService,
    ) -> Self {
        Self::for_caller(runtime_root, IpcCallerKind::DesktopShell, service)
    }

    fn for_caller(
        runtime_root: PathBuf,
        caller: IpcCallerKind,
        service: SecureStorageService,
    ) -> Self {
        Self {
            runtime_root,
            credentials: Arc::new(OsIpcCredentialReader::system(service.into_string())),
            caller,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
        }
    }

    /// 以方法需要的唯一作用域建立短会话并转发一次请求。
    ///
    /// Windows 上整个连接（建立、读写、丢弃）都放到一个专用线程上做，见 [`pipe_thread`]。
    ///
    /// # Errors
    ///
    /// Bridge 未启动、本地凭据不可用、超时或远端用例失败时返回可修复错误。
    pub async fn invoke(&self, method: IpcMethod) -> Result<IpcResponse, LocalBridgeClientFailure> {
        #[cfg(windows)]
        {
            let client = self.clone();
            pipe_thread::run(async move { client.invoke_here(method).await }).await
        }
        #[cfg(not(windows))]
        {
            self.invoke_here(method).await
        }
    }

    async fn invoke_here(
        &self,
        method: IpcMethod,
    ) -> Result<IpcResponse, LocalBridgeClientFailure> {
        let required_scope = method.required_scope();
        // Unix 套接字的路径与安装身份无关，先确认有 Bridge 在监听再读取凭据。Bridge 写好 IPC 凭据
        // 才开始监听，macOS 上读到的因此总是它重写过、本机程序可以直接读取的凭据，而不是旧版
        // Bridge 留下、读取时会弹窗要钥匙串密码的那份。Windows 的管道名取自安装身份，只能先读凭据。
        #[cfg(unix)]
        let stream = self
            .connect(&LocalIpcEndpoint::from_runtime_root(&self.runtime_root))
            .await?;
        let credentials = self
            .credentials
            .load()
            .map_err(LocalBridgeClientFailure::credential)?;
        #[cfg(not(unix))]
        let stream = self
            .connect(&LocalIpcEndpoint::from_installation(
                &self.runtime_root,
                credentials.installation_id(),
            ))
            .await?;
        let mut client = timeout(
            self.operation_timeout,
            IpcClientSession::authenticate(stream, &credentials, self.caller, [required_scope]),
        )
        .await
        .map_err(|_| LocalBridgeClientFailure::timeout())?
        .map_err(|failure| LocalBridgeClientFailure::ipc(&failure))?;
        self.request(&mut client, method).await
    }

    async fn connect(
        &self,
        endpoint: &LocalIpcEndpoint,
    ) -> Result<Stream, LocalBridgeClientFailure> {
        let name = endpoint
            .to_name()
            .map_err(|_| LocalBridgeClientFailure::endpoint())?;
        timeout(self.connect_timeout, Stream::connect(name))
            .await
            .map_err(|_| LocalBridgeClientFailure::timeout())?
            .map_err(|_| LocalBridgeClientFailure::unavailable())
    }

    async fn request<S>(
        &self,
        client: &mut IpcClientSession<S>,
        method: IpcMethod,
    ) -> Result<IpcResponse, LocalBridgeClientFailure>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let deadline = if matches!(method, IpcMethod::CloseHostSession(_)) {
            CLOSE_SESSION_TIMEOUT
        } else if method.name() == "matrix_recovery" {
            Duration::from_secs(90)
        } else {
            self.operation_timeout
        };
        timeout(deadline, client.request(method))
            .await
            .map_err(|_| LocalBridgeClientFailure::timeout())?
            .map_err(|failure| LocalBridgeClientFailure::ipc(&failure))
    }
}

/// Windows 具名管道客户端在一个线程上丢弃连接、运行时的 I/O 驱动同时在另一个线程上处理
/// 同一个管道时会踩坏堆，进程无声退出（`0xC0000374` / `0xC0000005`，上游 mio#2011）。
/// CLI 用单线程运行时规避；MCP 和桌面壳需要多线程并发，就把每个连接整个交给这一个专用线程：
/// 建立、读写、丢弃和驱动它的 I/O 都在同一线程上，两者不会重叠。调用方取消时这边的任务也被
/// 取消，连接仍在专用线程上丢弃。
#[cfg(windows)]
mod pipe_thread {
    use std::{future::Future, sync::LazyLock};

    use tokio::{
        runtime::{Builder, Handle},
        task::JoinHandle,
    };

    use super::LocalBridgeClientFailure;

    static HANDLE: LazyLock<Option<Handle>> = LazyLock::new(|| {
        let runtime = Builder::new_current_thread().enable_all().build().ok()?;
        let handle = runtime.handle().clone();
        std::thread::Builder::new()
            .name("agent-room-ipc".to_owned())
            .spawn(move || runtime.block_on(std::future::pending::<()>()))
            .ok()?;
        Some(handle)
    });

    struct AbortOnDrop<T>(JoinHandle<T>);

    impl<T> Drop for AbortOnDrop<T> {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    pub(super) async fn run<F, T>(work: F) -> Result<T, LocalBridgeClientFailure>
    where
        F: Future<Output = Result<T, LocalBridgeClientFailure>> + Send + 'static,
        T: Send + 'static,
    {
        // 专用线程起不来时（极少见）只能就地执行，行为与以前一样。
        let Some(handle) = HANDLE.as_ref() else {
            return work.await;
        };
        let mut task = AbortOnDrop(handle.spawn(work));
        (&mut task.0)
            .await
            .unwrap_or_else(|_| Err(LocalBridgeClientFailure::unavailable()))
    }

    #[cfg(test)]
    mod tests {
        use std::{
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
            time::Duration,
        };

        use super::run;

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn 连接在专用线程上跑_调用方取消时那边也取消() {
            let thread = run(async { Ok(std::thread::current().name().map(str::to_owned)) })
                .await
                .expect("专用线程可用");
            assert_eq!(thread.as_deref(), Some("agent-room-ipc"));

            let finished = Arc::new(AtomicBool::new(false));
            let slow = {
                let finished = finished.clone();
                run(async move {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    finished.store(true, Ordering::SeqCst);
                    Ok(())
                })
            };
            assert!(
                tokio::time::timeout(Duration::from_millis(50), slow)
                    .await
                    .is_err()
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(
                !finished.load(Ordering::SeqCst),
                "取消后专用线程上的任务不应继续跑完"
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalBridgeClientFailureKind {
    CredentialsMissing,
    CredentialsUnavailable,
    CredentialsCorrupt,
    EndpointInvalid,
    BridgeUnavailable,
    Timeout,
    Validation,
    Authentication,
    Authorization,
    IncompatibleVersion,
    Protocol,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalBridgeClientFailure {
    kind: LocalBridgeClientFailureKind,
    code: String,
    category: IpcErrorCategory,
    retryable: bool,
    details: BTreeMap<String, String>,
}

impl LocalBridgeClientFailure {
    fn new(
        kind: LocalBridgeClientFailureKind,
        code: impl Into<String>,
        category: IpcErrorCategory,
        retryable: bool,
    ) -> Self {
        Self {
            kind,
            code: code.into(),
            category,
            retryable,
            details: BTreeMap::new(),
        }
    }

    fn credential(failure: IpcCredentialFailure) -> Self {
        match failure.kind() {
            IpcCredentialFailureKind::Missing => Self::new(
                LocalBridgeClientFailureKind::CredentialsMissing,
                "bridge.ipc.credentials_missing",
                IpcErrorCategory::DependencyUnavailable,
                false,
            ),
            IpcCredentialFailureKind::Unavailable => Self::new(
                LocalBridgeClientFailureKind::CredentialsUnavailable,
                "bridge.ipc.credentials_unavailable",
                IpcErrorCategory::DependencyUnavailable,
                true,
            ),
            IpcCredentialFailureKind::Corrupt => Self::new(
                LocalBridgeClientFailureKind::CredentialsCorrupt,
                "bridge.ipc.credentials_corrupt",
                IpcErrorCategory::Authentication,
                false,
            ),
        }
    }

    fn endpoint() -> Self {
        Self::new(
            LocalBridgeClientFailureKind::EndpointInvalid,
            "bridge.ipc.endpoint_invalid",
            IpcErrorCategory::Internal,
            false,
        )
    }

    fn unavailable() -> Self {
        Self::new(
            LocalBridgeClientFailureKind::BridgeUnavailable,
            "bridge.ipc.bridge_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        )
    }

    fn timeout() -> Self {
        Self::new(
            LocalBridgeClientFailureKind::Timeout,
            "bridge.ipc.timeout",
            IpcErrorCategory::DependencyUnavailable,
            true,
        )
    }

    fn ipc(failure: &IpcClientFailure) -> Self {
        let kind = match failure.kind() {
            IpcClientFailureKind::Validation => LocalBridgeClientFailureKind::Validation,
            IpcClientFailureKind::Protocol | IpcClientFailureKind::InvalidHandshake => {
                LocalBridgeClientFailureKind::Protocol
            }
            IpcClientFailureKind::Authentication => LocalBridgeClientFailureKind::Authentication,
            IpcClientFailureKind::Authorization => LocalBridgeClientFailureKind::Authorization,
            IpcClientFailureKind::IncompatibleVersion => {
                LocalBridgeClientFailureKind::IncompatibleVersion
            }
            IpcClientFailureKind::Remote => LocalBridgeClientFailureKind::Remote,
        };
        Self {
            kind,
            code: failure.code().to_owned(),
            category: failure.category(),
            retryable: failure.retryable(),
            details: failure.details().clone(),
        }
    }

    pub const fn kind(&self) -> LocalBridgeClientFailureKind {
        self.kind
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub const fn category(&self) -> IpcErrorCategory {
        self.category
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    pub const fn details(&self) -> &BTreeMap<String, String> {
        &self.details
    }
}

#[cfg(test)]
mod tests;
