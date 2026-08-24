use std::sync::Arc;

use agent_room_application::ports::{
    AgentCardFetchFailure, AgentCardFetchFailureKind, AgentCardFetchResult, AgentCardSource,
    FetchedAgentCard, PortFuture,
};
use agent_room_domain::agent_cards::{AgentCardSourceUrl, AgentCardVerificationState};

use crate::{
    AgentCardNormalizationFailure, AgentCardNormalizationFailureKind, AgentCardNormalizer,
    AgentCardSignatureFailure, AgentCardSignatureFailureKind, AgentCardSignatureVerifier,
    HttpsDocumentClient, NetworkTargetFailure, NetworkTargetFailureKind,
};

const MAXIMUM_AGENT_CARD_BYTES: usize = 65_536;

pub struct RemoteAgentCardSource {
    documents: Arc<dyn HttpsDocumentClient>,
    normalizer: AgentCardNormalizer,
    signatures: AgentCardSignatureVerifier,
}

impl RemoteAgentCardSource {
    pub fn new(documents: Arc<dyn HttpsDocumentClient>, normalizer: AgentCardNormalizer) -> Self {
        Self {
            documents: Arc::clone(&documents),
            normalizer,
            signatures: AgentCardSignatureVerifier::new(documents),
        }
    }

    async fn fetch_internal(
        &self,
        source_url: &AgentCardSourceUrl,
    ) -> AgentCardFetchResult<FetchedAgentCard> {
        let document = self
            .documents
            .get_json(source_url, MAXIMUM_AGENT_CARD_BYTES)
            .await
            .map_err(map_document_failure)?;
        let parsed = self
            .normalizer
            .parse(&document, source_url)
            .map_err(map_normalization_failure)?;
        let verification = if parsed.has_signatures() {
            self.signatures
                .verify(&parsed, source_url)
                .await
                .map_err(map_signature_failure)?;
            AgentCardVerificationState::Verified
        } else {
            AgentCardVerificationState::Unverified
        };
        let (digest, card) = parsed.into_parts();
        Ok(FetchedAgentCard {
            digest,
            card,
            verification,
            cache_lifetime: document.cache_lifetime(),
        })
    }
}

impl AgentCardSource for RemoteAgentCardSource {
    fn fetch<'a>(
        &'a self,
        source_url: &'a AgentCardSourceUrl,
    ) -> PortFuture<'a, AgentCardFetchResult<FetchedAgentCard>> {
        Box::pin(async move { self.fetch_internal(source_url).await })
    }
}

fn map_document_failure(failure: NetworkTargetFailure) -> AgentCardFetchFailure {
    let kind = match failure.kind() {
        NetworkTargetFailureKind::InvalidTarget => AgentCardFetchFailureKind::RejectedSource,
        NetworkTargetFailureKind::BlockedAddress => AgentCardFetchFailureKind::BlockedNetworkTarget,
        NetworkTargetFailureKind::ResolutionFailed | NetworkTargetFailureKind::ConnectFailed => {
            AgentCardFetchFailureKind::Unavailable
        }
        NetworkTargetFailureKind::InvalidResponse | NetworkTargetFailureKind::ResponseTooLarge => {
            AgentCardFetchFailureKind::InvalidResponse
        }
        NetworkTargetFailureKind::Internal => AgentCardFetchFailureKind::Internal,
    };
    AgentCardFetchFailure::new("a2a.agent_card.fetch_document", kind)
}

fn map_normalization_failure(failure: AgentCardNormalizationFailure) -> AgentCardFetchFailure {
    let kind = match failure.kind() {
        AgentCardNormalizationFailureKind::InvalidJson
        | AgentCardNormalizationFailureKind::InvalidSchema
        | AgentCardNormalizationFailureKind::InvalidSecurityScheme => {
            AgentCardFetchFailureKind::InvalidResponse
        }
        AgentCardNormalizationFailureKind::UnsupportedProtocol
        | AgentCardNormalizationFailureKind::UnsupportedRequiredExtension => {
            AgentCardFetchFailureKind::UnsupportedProtocol
        }
        AgentCardNormalizationFailureKind::Internal => AgentCardFetchFailureKind::Internal,
    };
    AgentCardFetchFailure::new("a2a.agent_card.normalize_document", kind)
}

fn map_signature_failure(failure: AgentCardSignatureFailure) -> AgentCardFetchFailure {
    let kind = match failure.kind() {
        AgentCardSignatureFailureKind::InvalidSignature => {
            AgentCardFetchFailureKind::InvalidSignature
        }
        AgentCardSignatureFailureKind::BlockedNetworkTarget => {
            AgentCardFetchFailureKind::BlockedNetworkTarget
        }
        AgentCardSignatureFailureKind::Unavailable => AgentCardFetchFailureKind::Unavailable,
        AgentCardSignatureFailureKind::Internal => AgentCardFetchFailureKind::Internal,
    };
    AgentCardFetchFailure::new("a2a.agent_card.verify_signature", kind)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_room_application::ports::{AgentCardSource as _, PortFuture};
    use agent_room_domain::{
        agent_cards::{AgentCardSourceUrl, AgentCardVerificationState},
        time::DurationMillis,
    };

    use super::{AgentCardNormalizer, RemoteAgentCardSource};
    use crate::{HttpsDocumentClient, JsonDocument, NetworkTargetResult};

    struct FixtureClient;

    impl HttpsDocumentClient for FixtureClient {
        fn get_json<'a>(
            &'a self,
            _source_url: &'a AgentCardSourceUrl,
            _maximum_bytes: usize,
        ) -> PortFuture<'a, NetworkTargetResult<JsonDocument>> {
            Box::pin(async {
                Ok(JsonDocument::new(
                    include_bytes!("../fixtures/a2a-1.0-agent-card.json").to_vec(),
                    DurationMillis::new(120_000).expect("测试缓存时限有效"),
                ))
            })
        }
    }

    #[tokio::test]
    async fn 未签名_agent_card_以未验证状态返回() {
        let source =
            RemoteAgentCardSource::new(Arc::new(FixtureClient), AgentCardNormalizer::default());
        let fetched = source
            .fetch(&source_url())
            .await
            .expect("有效未签名资料应被读取");

        assert_eq!(fetched.verification, AgentCardVerificationState::Unverified);
        assert_eq!(fetched.cache_lifetime.value(), 120_000);
        assert_eq!(fetched.card.name(), "Route Planner");
    }

    fn source_url() -> AgentCardSourceUrl {
        AgentCardSourceUrl::new("https://agent.example/.well-known/agent-card.json".to_owned())
            .expect("测试来源有效")
    }
}
