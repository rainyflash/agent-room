mod failure;
mod models;
mod service;

pub use failure::{
    AutomationFailure, AutomationFailureKind, AutomationResult, AutomationSendDenial,
};
pub use models::{
    AuthorizeAutomationSend, AutomationAuthorizationOutcome, AutomationAuthorizationReceipt,
    CreateAutomationGrant, ListAutomationGrants, RevokeAutomationGrant,
};
pub use service::{AutomationDependencies, AutomationService, AutomationUseCases};
