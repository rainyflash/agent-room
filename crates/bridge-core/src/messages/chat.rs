/// 在传输层提前校验聊天，不向 IPC 暴露领域类型。
///
/// # Errors
///
/// 聊天内容或提及无效时返回错误。
pub fn validate_chat(
    text: &str,
    mentions: &[String],
) -> Result<(), agent_room_domain::error::DomainError> {
    agent_room_domain::messages::ConversationMessage::new(text.to_owned(), mentions.to_vec())
        .map(|_| ())
}
