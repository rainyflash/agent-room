use std::sync::Arc;

use crate::ports::PortFuture;

use super::{
    BeginContentUploadOutcome, BeginContentUploadRequest, BeginContentUploadResult,
    BeginContentUploadService, BindContentEventOutcome, BindContentEventRequest,
    BindContentEventResult, BindContentEventService, CompleteContentUploadOutcome,
    CompleteContentUploadRequest, CompleteContentUploadResult, CompleteContentUploadService,
    IssueContentReadTicketRequest, IssueContentReadTicketResult, IssueContentReadTicketService,
    IssuedContentReadTicket, OpenContentRequest, OpenContentResult, OpenContentService,
    OpenedVerifiedContent, RedactContentOutcome, RedactContentRequest, RedactContentResult,
    RedactContentService,
};

/// 内容 HTTP、MCP 与后续客户端适配器共享的应用能力边界。
pub trait ContentUseCases: Send + Sync {
    fn begin_upload(
        &self,
        request: BeginContentUploadRequest,
    ) -> PortFuture<'_, BeginContentUploadResult<BeginContentUploadOutcome>>;

    fn complete_upload(
        &self,
        request: CompleteContentUploadRequest,
    ) -> PortFuture<'_, CompleteContentUploadResult<CompleteContentUploadOutcome>>;

    fn bind_event(
        &self,
        request: BindContentEventRequest,
    ) -> PortFuture<'_, BindContentEventResult<BindContentEventOutcome>>;

    fn redact(
        &self,
        request: RedactContentRequest,
    ) -> PortFuture<'_, RedactContentResult<RedactContentOutcome>>;

    fn issue_read_ticket(
        &self,
        request: IssueContentReadTicketRequest,
    ) -> PortFuture<'_, IssueContentReadTicketResult<IssuedContentReadTicket>>;

    fn open(
        &self,
        request: OpenContentRequest,
    ) -> PortFuture<'_, OpenContentResult<OpenedVerifiedContent>>;
}

pub struct ContentServiceDependencies {
    pub begin_upload: Arc<BeginContentUploadService>,
    pub complete_upload: Arc<CompleteContentUploadService>,
    pub bind_event: Arc<BindContentEventService>,
    pub redact: Arc<RedactContentService>,
    pub issue_read_ticket: Arc<IssueContentReadTicketService>,
    pub open: Arc<OpenContentService>,
}

pub struct ContentService {
    begin_upload: Arc<BeginContentUploadService>,
    complete_upload: Arc<CompleteContentUploadService>,
    bind_event: Arc<BindContentEventService>,
    redact: Arc<RedactContentService>,
    issue_read_ticket: Arc<IssueContentReadTicketService>,
    open: Arc<OpenContentService>,
}

impl ContentService {
    pub fn new(dependencies: ContentServiceDependencies) -> Self {
        Self {
            begin_upload: dependencies.begin_upload,
            complete_upload: dependencies.complete_upload,
            bind_event: dependencies.bind_event,
            redact: dependencies.redact,
            issue_read_ticket: dependencies.issue_read_ticket,
            open: dependencies.open,
        }
    }
}

impl ContentUseCases for ContentService {
    fn begin_upload(
        &self,
        request: BeginContentUploadRequest,
    ) -> PortFuture<'_, BeginContentUploadResult<BeginContentUploadOutcome>> {
        Box::pin(self.begin_upload.begin(request))
    }

    fn complete_upload(
        &self,
        request: CompleteContentUploadRequest,
    ) -> PortFuture<'_, CompleteContentUploadResult<CompleteContentUploadOutcome>> {
        Box::pin(self.complete_upload.complete(request))
    }

    fn bind_event(
        &self,
        request: BindContentEventRequest,
    ) -> PortFuture<'_, BindContentEventResult<BindContentEventOutcome>> {
        Box::pin(self.bind_event.bind(request))
    }

    fn redact(
        &self,
        request: RedactContentRequest,
    ) -> PortFuture<'_, RedactContentResult<RedactContentOutcome>> {
        Box::pin(self.redact.redact(request))
    }

    fn issue_read_ticket(
        &self,
        request: IssueContentReadTicketRequest,
    ) -> PortFuture<'_, IssueContentReadTicketResult<IssuedContentReadTicket>> {
        Box::pin(self.issue_read_ticket.issue(request))
    }

    fn open(
        &self,
        request: OpenContentRequest,
    ) -> PortFuture<'_, OpenContentResult<OpenedVerifiedContent>> {
        Box::pin(self.open.open(request))
    }
}
