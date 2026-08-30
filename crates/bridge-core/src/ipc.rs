use std::collections::BTreeSet;

const MAX_ADVERTISED_VERSIONS: usize = 4;
const MAX_REQUESTED_SCOPES: usize = 16;
const MAX_INSTALLATION_ID_LENGTH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IpcInstallationId(String);

impl IpcInstallationId {
    /// 构造不包含用户路径或设备秘密的安装标识。
    ///
    /// # Errors
    ///
    /// 标识为空、超长或包含非安全字符时返回校验错误。
    pub fn new(value: impl Into<String>) -> IpcHandshakeResult<Self> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_INSTALLATION_ID_LENGTH
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(IpcHandshakeFailure::new(
                "bridge.ipc.validate_installation_id",
                IpcHandshakeFailureKind::InvalidOffer,
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IpcProtocolVersion {
    major: u16,
    minor: u16,
}

impl IpcProtocolVersion {
    pub const V1_0: Self = Self { major: 1, minor: 0 };

    /// 构造本地 IPC 协议版本。
    ///
    /// # Errors
    ///
    /// 主版本为零时返回校验错误。
    pub const fn new(major: u16, minor: u16) -> Result<Self, IpcHandshakeFailure> {
        if major == 0 {
            return Err(IpcHandshakeFailure::new(
                "bridge.ipc.validate_version",
                IpcHandshakeFailureKind::InvalidOffer,
            ));
        }
        Ok(Self { major, minor })
    }

    pub const fn major(self) -> u16 {
        self.major
    }

    pub const fn minor(self) -> u16 {
        self.minor
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IpcCallerKind {
    McpServer,
    DesktopShell,
    DiagnosticCli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IpcScope {
    BridgeStatusRead,
    SelfRead,
    AgentBootstrap,
    PreviewsRead,
    PresenceRead,
    ContentRead,
    StatusPublish,
    MessageSend,
    HandoffApprove,
    HandoffList,
    HandoffConsume,
    HandoffDecline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcHandshakeOffer {
    caller: IpcCallerKind,
    supported_versions: BTreeSet<IpcProtocolVersion>,
    requested_scopes: BTreeSet<IpcScope>,
}

impl IpcHandshakeOffer {
    /// 创建已经完成传输层身份校验的握手提议。
    ///
    /// # Errors
    ///
    /// 版本或作用域为空、数量超限时返回校验错误。
    pub fn new(
        caller: IpcCallerKind,
        supported_versions: impl IntoIterator<Item = IpcProtocolVersion>,
        requested_scopes: impl IntoIterator<Item = IpcScope>,
    ) -> IpcHandshakeResult<Self> {
        let supported_versions = supported_versions.into_iter().collect::<BTreeSet<_>>();
        let requested_scopes = requested_scopes.into_iter().collect::<BTreeSet<_>>();
        if supported_versions.is_empty()
            || supported_versions.len() > MAX_ADVERTISED_VERSIONS
            || requested_scopes.is_empty()
            || requested_scopes.len() > MAX_REQUESTED_SCOPES
        {
            return Err(IpcHandshakeFailure::new(
                "bridge.ipc.validate_offer",
                IpcHandshakeFailureKind::InvalidOffer,
            ));
        }
        Ok(Self {
            caller,
            supported_versions,
            requested_scopes,
        })
    }

    pub const fn caller(&self) -> IpcCallerKind {
        self.caller
    }

    pub const fn supported_versions(&self) -> &BTreeSet<IpcProtocolVersion> {
        &self.supported_versions
    }

    pub const fn requested_scopes(&self) -> &BTreeSet<IpcScope> {
        &self.requested_scopes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcHandshakeAgreement {
    selected_version: IpcProtocolVersion,
    granted_scopes: BTreeSet<IpcScope>,
}

impl IpcHandshakeAgreement {
    /// 校验服务端返回的版本与作用域选择。
    ///
    /// # Errors
    ///
    /// 版本未在客户端提议中、授权为空或包含未申请作用域时返回稳定错误。
    pub fn from_server_selection(
        offer: &IpcHandshakeOffer,
        selected_version: IpcProtocolVersion,
        granted_scopes: impl IntoIterator<Item = IpcScope>,
    ) -> IpcHandshakeResult<Self> {
        if !offer.supported_versions().contains(&selected_version) {
            return Err(IpcHandshakeFailure::new(
                "bridge.ipc.accept_version",
                IpcHandshakeFailureKind::IncompatibleVersion,
            ));
        }
        let granted_scopes = granted_scopes.into_iter().collect::<BTreeSet<_>>();
        if granted_scopes.is_empty()
            || granted_scopes.len() > MAX_REQUESTED_SCOPES
            || !granted_scopes.is_subset(offer.requested_scopes())
        {
            return Err(IpcHandshakeFailure::new(
                "bridge.ipc.accept_scopes",
                IpcHandshakeFailureKind::ScopeDenied,
            ));
        }
        Ok(Self {
            selected_version,
            granted_scopes,
        })
    }

    pub const fn selected_version(&self) -> IpcProtocolVersion {
        self.selected_version
    }

    pub const fn granted_scopes(&self) -> &BTreeSet<IpcScope> {
        &self.granted_scopes
    }
}

pub trait IpcScopePolicy: Send + Sync {
    fn allows(&self, caller: IpcCallerKind, scope: IpcScope) -> bool;
}

#[derive(Debug, Default)]
pub struct FoundationIpcScopePolicy;

impl IpcScopePolicy for FoundationIpcScopePolicy {
    fn allows(&self, caller: IpcCallerKind, scope: IpcScope) -> bool {
        match caller {
            IpcCallerKind::McpServer => {
                !matches!(scope, IpcScope::AgentBootstrap | IpcScope::HandoffApprove)
            }
            IpcCallerKind::DesktopShell => {
                matches!(
                    scope,
                    IpcScope::BridgeStatusRead
                        | IpcScope::SelfRead
                        | IpcScope::AgentBootstrap
                        | IpcScope::PreviewsRead
                        | IpcScope::PresenceRead
                        | IpcScope::HandoffApprove
                )
            }
            IpcCallerKind::DiagnosticCli => scope == IpcScope::BridgeStatusRead,
        }
    }
}

pub struct IpcHandshakeNegotiator<P> {
    supported_versions: BTreeSet<IpcProtocolVersion>,
    scope_policy: P,
}

impl<P> IpcHandshakeNegotiator<P>
where
    P: IpcScopePolicy,
{
    /// 创建服务端协商器。
    ///
    /// # Errors
    ///
    /// 服务端版本集合为空或数量超限时返回配置错误。
    pub fn new(
        supported_versions: impl IntoIterator<Item = IpcProtocolVersion>,
        scope_policy: P,
    ) -> IpcHandshakeResult<Self> {
        let supported_versions = supported_versions.into_iter().collect::<BTreeSet<_>>();
        if supported_versions.is_empty() || supported_versions.len() > MAX_ADVERTISED_VERSIONS {
            return Err(IpcHandshakeFailure::new(
                "bridge.ipc.configure",
                IpcHandshakeFailureKind::InvalidConfiguration,
            ));
        }
        Ok(Self {
            supported_versions,
            scope_policy,
        })
    }

    /// 在调用方已经通过安装身份挑战后协商版本与最小作用域。
    ///
    /// # Errors
    ///
    /// 没有共同版本或请求了调用方不允许的作用域时返回稳定错误。
    pub fn negotiate(
        &self,
        offer: &IpcHandshakeOffer,
    ) -> IpcHandshakeResult<IpcHandshakeAgreement> {
        let selected_version = self
            .supported_versions
            .intersection(offer.supported_versions())
            .max()
            .copied()
            .ok_or_else(|| {
                IpcHandshakeFailure::new(
                    "bridge.ipc.negotiate_version",
                    IpcHandshakeFailureKind::IncompatibleVersion,
                )
            })?;
        if offer
            .requested_scopes()
            .iter()
            .any(|scope| !self.scope_policy.allows(offer.caller(), *scope))
        {
            return Err(IpcHandshakeFailure::new(
                "bridge.ipc.authorize_scopes",
                IpcHandshakeFailureKind::ScopeDenied,
            ));
        }

        IpcHandshakeAgreement::from_server_selection(
            offer,
            selected_version,
            offer.requested_scopes().iter().copied(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcHandshakeFailureKind {
    InvalidConfiguration,
    InvalidOffer,
    AuthenticationRejected,
    IncompatibleVersion,
    ScopeDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcHandshakeFailure {
    operation: &'static str,
    kind: IpcHandshakeFailureKind,
}

impl IpcHandshakeFailure {
    pub const fn new(operation: &'static str, kind: IpcHandshakeFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> IpcHandshakeFailureKind {
        self.kind
    }
}

pub type IpcHandshakeResult<T> = Result<T, IpcHandshakeFailure>;

#[cfg(test)]
mod tests {
    use super::{
        FoundationIpcScopePolicy, IpcCallerKind, IpcHandshakeAgreement, IpcHandshakeFailureKind,
        IpcHandshakeNegotiator, IpcHandshakeOffer, IpcProtocolVersion, IpcScope, IpcScopePolicy,
    };

    #[test]
    fn 协商选择双方共同支持的最高版本() {
        let negotiator = IpcHandshakeNegotiator::new(
            [
                IpcProtocolVersion::V1_0,
                IpcProtocolVersion::new(1, 1).expect("测试版本有效"),
            ],
            FoundationIpcScopePolicy,
        )
        .expect("协商器配置有效");
        let offer = IpcHandshakeOffer::new(
            IpcCallerKind::McpServer,
            [
                IpcProtocolVersion::V1_0,
                IpcProtocolVersion::new(1, 1).expect("测试版本有效"),
            ],
            [IpcScope::BridgeStatusRead],
        )
        .expect("握手提议有效");

        let agreement = negotiator.negotiate(&offer).expect("共同版本可协商");

        assert_eq!(
            agreement.selected_version(),
            IpcProtocolVersion::new(1, 1).expect("测试版本有效")
        );
        assert_eq!(agreement.granted_scopes(), offer.requested_scopes());
    }

    #[test]
    fn 不兼容版本明确失败而不是静默降级() {
        let negotiator =
            IpcHandshakeNegotiator::new([IpcProtocolVersion::V1_0], FoundationIpcScopePolicy)
                .expect("协商器配置有效");
        let offer = IpcHandshakeOffer::new(
            IpcCallerKind::DesktopShell,
            [IpcProtocolVersion::new(2, 0).expect("测试版本有效")],
            [IpcScope::BridgeStatusRead],
        )
        .expect("握手提议有效");

        let failure = negotiator
            .negotiate(&offer)
            .expect_err("不兼容版本必须失败");

        assert_eq!(failure.kind(), IpcHandshakeFailureKind::IncompatibleVersion);
    }

    #[test]
    fn 空版本或空作用域不是合法握手() {
        assert_eq!(
            IpcHandshakeOffer::new(
                IpcCallerKind::DiagnosticCli,
                [],
                [IpcScope::BridgeStatusRead]
            )
            .expect_err("空版本必须失败")
            .kind(),
            IpcHandshakeFailureKind::InvalidOffer
        );
        assert_eq!(
            IpcHandshakeOffer::new(IpcCallerKind::DiagnosticCli, [IpcProtocolVersion::V1_0], [])
                .expect_err("空作用域必须失败")
                .kind(),
            IpcHandshakeFailureKind::InvalidOffer
        );
    }

    struct 拒绝全部作用域;

    impl IpcScopePolicy for 拒绝全部作用域 {
        fn allows(&self, _caller: IpcCallerKind, _scope: IpcScope) -> bool {
            false
        }
    }

    #[test]
    fn 作用域必须由调用方策略显式允许() {
        let negotiator = IpcHandshakeNegotiator::new([IpcProtocolVersion::V1_0], 拒绝全部作用域)
            .expect("协商器配置有效");
        let offer = IpcHandshakeOffer::new(
            IpcCallerKind::McpServer,
            [IpcProtocolVersion::V1_0],
            [IpcScope::BridgeStatusRead],
        )
        .expect("握手提议有效");

        let failure = negotiator
            .negotiate(&offer)
            .expect_err("未授权作用域必须失败");

        assert_eq!(failure.kind(), IpcHandshakeFailureKind::ScopeDenied);
    }

    #[test]
    fn mcp_server_可逐项申请工具作用域而诊断客户端只能读取状态() {
        let policy = FoundationIpcScopePolicy;
        let tool_scopes = [
            IpcScope::SelfRead,
            IpcScope::PreviewsRead,
            IpcScope::PresenceRead,
            IpcScope::ContentRead,
            IpcScope::StatusPublish,
            IpcScope::MessageSend,
            IpcScope::HandoffList,
            IpcScope::HandoffConsume,
            IpcScope::HandoffDecline,
        ];

        assert!(
            tool_scopes
                .iter()
                .all(|scope| policy.allows(IpcCallerKind::McpServer, *scope))
        );
        assert!(
            tool_scopes
                .iter()
                .all(|scope| !policy.allows(IpcCallerKind::DiagnosticCli, *scope))
        );
        assert!(policy.allows(IpcCallerKind::DiagnosticCli, IpcScope::BridgeStatusRead));
        assert!(policy.allows(IpcCallerKind::DesktopShell, IpcScope::SelfRead));
        assert!(policy.allows(IpcCallerKind::DesktopShell, IpcScope::AgentBootstrap));
        assert!(policy.allows(IpcCallerKind::DesktopShell, IpcScope::PreviewsRead));
        assert!(policy.allows(IpcCallerKind::DesktopShell, IpcScope::PresenceRead));
        assert!(policy.allows(IpcCallerKind::DesktopShell, IpcScope::HandoffApprove));
        assert!(!policy.allows(IpcCallerKind::DesktopShell, IpcScope::ContentRead));
        assert!(!policy.allows(IpcCallerKind::DesktopShell, IpcScope::MessageSend));
        assert!(!policy.allows(IpcCallerKind::McpServer, IpcScope::HandoffApprove));
        assert!(!policy.allows(IpcCallerKind::McpServer, IpcScope::AgentBootstrap));
        assert!(!policy.allows(IpcCallerKind::DiagnosticCli, IpcScope::HandoffApprove));
    }

    #[test]
    fn 客户端拒绝未提议版本与越权作用域() {
        let offer = IpcHandshakeOffer::new(
            IpcCallerKind::McpServer,
            [IpcProtocolVersion::V1_0],
            [IpcScope::SelfRead],
        )
        .expect("握手提议有效");

        let version_failure = IpcHandshakeAgreement::from_server_selection(
            &offer,
            IpcProtocolVersion::new(2, 0).expect("测试版本有效"),
            [IpcScope::SelfRead],
        )
        .expect_err("未提议版本必须失败");
        assert_eq!(
            version_failure.kind(),
            IpcHandshakeFailureKind::IncompatibleVersion
        );

        let scope_failure = IpcHandshakeAgreement::from_server_selection(
            &offer,
            IpcProtocolVersion::V1_0,
            [IpcScope::MessageSend],
        )
        .expect_err("未申请作用域必须失败");
        assert_eq!(scope_failure.kind(), IpcHandshakeFailureKind::ScopeDenied);
    }
}
