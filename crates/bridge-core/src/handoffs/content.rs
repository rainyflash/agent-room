use std::sync::Arc;

use agent_room_application::ports::MatrixRoomId;
use agent_room_domain::{
    handoff::{ContextHandoff, HandoffPermission},
    messages::MessageContentReference,
};

use crate::messages::{
    MessageContentReadFailureKind, MessageContentReadGateway, MessageContentReadRequest,
    MessageContentSourceQuery, MessageTimelineQueryFailureKind, MessageTimelineQueryRepository,
    ProjectedMessagePreview,
};

use super::{
    HandoffContentFailure, HandoffContentFailureKind, HandoffContentGateway, HandoffContentRead,
};

pub struct ProjectedHandoffContentGateway {
    projections: Arc<dyn MessageTimelineQueryRepository>,
    content: Arc<dyn MessageContentReadGateway>,
}

impl ProjectedHandoffContentGateway {
    pub const fn new(
        projections: Arc<dyn MessageTimelineQueryRepository>,
        content: Arc<dyn MessageContentReadGateway>,
    ) -> Self {
        Self {
            projections,
            content,
        }
    }

    async fn read_internal(
        &self,
        handoff: &ContextHandoff,
    ) -> Result<HandoffContentRead, HandoffContentFailure> {
        let fields = handoff.fields();
        let room_id = MatrixRoomId::new(fields.source.room_id().as_str().to_owned())
            .map_err(|_| failure(HandoffContentFailureKind::InvalidResponse))?;
        let source = self
            .projections
            .find_content_source(&MessageContentSourceQuery::new(
                room_id,
                fields.content.content_id(),
            ))
            .await
            .map_err(map_projection_failure)?
            .ok_or_else(|| failure(HandoffContentFailureKind::NotFound))?;
        if !source_matches_handoff(&source, handoff) || !scope_allows_content(handoff) {
            return Err(failure(HandoffContentFailureKind::Denied));
        }
        let opened = self
            .content
            .open(&MessageContentReadRequest::new(
                fields.content.content_id(),
                fields.content.byte_length().value(),
            ))
            .await
            .map_err(map_content_failure)?;
        if opened.byte_length != fields.content.byte_length()
            || opened.digest != fields.content.digest()
            || opened.media_type != *fields.content.media_type()
            || u64::try_from(opened.bytes.len()).ok() != Some(fields.content.byte_length().value())
        {
            return Err(failure(HandoffContentFailureKind::InvalidResponse));
        }
        Ok(HandoffContentRead {
            body: opened.bytes,
            media_type: opened.media_type,
        })
    }
}

impl HandoffContentGateway for ProjectedHandoffContentGateway {
    fn read<'a>(
        &'a self,
        handoff: &'a ContextHandoff,
    ) -> agent_room_application::ports::PortFuture<
        'a,
        Result<HandoffContentRead, HandoffContentFailure>,
    > {
        Box::pin(self.read_internal(handoff))
    }
}

fn source_matches_handoff(source: &ProjectedMessagePreview, handoff: &ContextHandoff) -> bool {
    let fields = handoff.fields();
    let expected_source = &fields.source;
    let expected_actor = expected_source.actor();
    let actual_actor = source.actor.identity();
    source.room_id.as_str() == expected_source.room_id().as_str()
        && source.event_id.as_str() == expected_source.event_id().as_str()
        && source.message_id == expected_source.message_id()
        && actual_actor.agent_id() == expected_actor.agent_id()
        && actual_actor.agent_instance_id() == expected_actor.instance_id()
        && source.actor.provenance() == expected_actor.provenance()
        && content_reference_matches(source.content, handoff)
        && source.preview.content_type() == fields.content.media_type()
        && &fields.risk_flags == source.preview.risk_flags()
}

fn content_reference_matches(actual: MessageContentReference, handoff: &ContextHandoff) -> bool {
    let expected = &handoff.fields().content;
    actual.content_id() == expected.content_id()
        && actual.digest() == expected.digest()
        && actual.size_bytes() == expected.byte_length().value()
}

fn scope_allows_content(handoff: &ContextHandoff) -> bool {
    let fields = handoff.fields();
    let media_type = fields.content.media_type().as_str();
    let is_text = media_type == "application/json" || media_type.starts_with("text/");
    fields.permissions.contains(if is_text {
        HandoffPermission::ReadText
    } else {
        HandoffPermission::ReadAttachments
    })
}

const fn map_projection_failure(
    projection_failure: crate::messages::MessageTimelineQueryFailure,
) -> HandoffContentFailure {
    failure(match projection_failure.kind() {
        MessageTimelineQueryFailureKind::Unavailable => HandoffContentFailureKind::Unavailable,
        MessageTimelineQueryFailureKind::CursorNotFound
        | MessageTimelineQueryFailureKind::Corrupt => HandoffContentFailureKind::InvalidResponse,
    })
}

const fn map_content_failure(
    content_failure: crate::messages::MessageContentReadFailure,
) -> HandoffContentFailure {
    failure(match content_failure.kind() {
        MessageContentReadFailureKind::NotFound => HandoffContentFailureKind::NotFound,
        MessageContentReadFailureKind::Denied => HandoffContentFailureKind::Denied,
        MessageContentReadFailureKind::RateLimited | MessageContentReadFailureKind::Unavailable => {
            HandoffContentFailureKind::Unavailable
        }
        MessageContentReadFailureKind::InvalidRequest
        | MessageContentReadFailureKind::InvalidResponse
        | MessageContentReadFailureKind::Internal => HandoffContentFailureKind::InvalidResponse,
    })
}

const fn failure(kind: HandoffContentFailureKind) -> HandoffContentFailure {
    HandoffContentFailure::new(kind)
}
