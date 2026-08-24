use agent_room_application::ports::{
    ContentStorageKeyFactory, ContentStorageKeyGenerationFailure, ContentStorageKeyGenerationResult,
};
use agent_room_domain::{content::ContentStorageKey, ids::ContentId};

use crate::encoding::lower_hex;

const RANDOM_SUFFIX_BYTES: usize = 32;

#[derive(Debug, Default, Clone, Copy)]
pub struct SecureContentStorageKeyFactory;

impl ContentStorageKeyFactory for SecureContentStorageKeyFactory {
    fn generate(
        &self,
        content_id: ContentId,
    ) -> ContentStorageKeyGenerationResult<ContentStorageKey> {
        let mut entropy = [0_u8; RANDOM_SUFFIX_BYTES];
        getrandom::fill(&mut entropy).map_err(|_| ContentStorageKeyGenerationFailure)?;
        let random_suffix = lower_hex(&entropy);
        let shard = &random_suffix[..2];
        ContentStorageKey::new(format!("content/v1/{shard}/{content_id}/{random_suffix}"))
            .map_err(|_| ContentStorageKeyGenerationFailure)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use agent_room_application::ports::ContentStorageKeyFactory;
    use agent_room_domain::ids::ContentId;
    use uuid::Uuid;

    use super::SecureContentStorageKeyFactory;

    #[test]
    fn 同一内容每次生成不同且不暴露用户语义的对象键() {
        let factory = SecureContentStorageKeyFactory;
        let content_id = ContentId::from_uuid(
            Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a3").expect("UUID 有效"),
        );
        let generated = (0..64)
            .map(|_| factory.generate(content_id).expect("随机源可用"))
            .collect::<HashSet<_>>();

        assert_eq!(generated.len(), 64);
        assert!(generated.iter().all(|key| {
            key.as_str().starts_with("content/v1/")
                && !key.as_str().contains("filename")
                && !key.as_str().contains("principal")
        }));
    }
}
