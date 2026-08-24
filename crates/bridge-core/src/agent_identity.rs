use agent_room_application::ports::MatrixUserId;
use agent_room_domain::ids::{AgentId, AgentInstanceId};

const MAX_DISPLAY_NAME_CHARACTERS: usize = 80;
const MAX_AVATAR_URL_LENGTH: usize = 2_048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAgentIdentity {
    agent_id: AgentId,
    display_name: String,
    matrix_user_id: MatrixUserId,
    agent_instance_id: AgentInstanceId,
    avatar_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAgentIdentityError {
    InvalidDisplayName,
    InvalidMatrixUserId,
    InvalidAvatarUrl,
}

impl BridgeAgentIdentity {
    /// 创建所有 Agent 协议事件共用的公开身份。
    ///
    /// # Errors
    ///
    /// 展示名、Matrix 用户标识或头像地址不满足协议边界时返回错误。
    pub fn new(
        agent_id: AgentId,
        display_name: impl Into<String>,
        matrix_user_id: impl Into<String>,
        agent_instance_id: AgentInstanceId,
    ) -> Result<Self, BridgeAgentIdentityError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty()
            || display_name.chars().count() > MAX_DISPLAY_NAME_CHARACTERS
            || display_name.chars().any(char::is_control)
        {
            return Err(BridgeAgentIdentityError::InvalidDisplayName);
        }
        let matrix_user_id = MatrixUserId::new(matrix_user_id.into())
            .map_err(|_| BridgeAgentIdentityError::InvalidMatrixUserId)?;
        Ok(Self {
            agent_id,
            display_name,
            matrix_user_id,
            agent_instance_id,
            avatar_url: None,
        })
    }

    /// 附加公开头像地址。
    ///
    /// # Errors
    ///
    /// 地址不是 HTTPS、超长或包含控制字符时返回错误。
    pub fn with_avatar_url(
        mut self,
        avatar_url: impl Into<String>,
    ) -> Result<Self, BridgeAgentIdentityError> {
        let avatar_url = avatar_url.into();
        if avatar_url.len() > MAX_AVATAR_URL_LENGTH
            || !avatar_url.starts_with("https://")
            || avatar_url.chars().any(char::is_control)
        {
            return Err(BridgeAgentIdentityError::InvalidAvatarUrl);
        }
        self.avatar_url = Some(avatar_url);
        Ok(self)
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub const fn matrix_user_id(&self) -> &MatrixUserId {
        &self.matrix_user_id
    }

    pub const fn agent_instance_id(&self) -> AgentInstanceId {
        self.agent_instance_id
    }

    pub fn avatar_url(&self) -> Option<&str> {
        self.avatar_url.as_deref()
    }
}
