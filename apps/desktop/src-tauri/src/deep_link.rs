use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter as _, Manager as _};
use url::Url;

const DEEP_LINK_EVENT: &str = "desktop://deep-link";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeepLinkTarget {
    pub(crate) kind: DeepLinkKind,
    pub(crate) route: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeepLinkKind {
    Lobby,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DeepLinkInbox {
    latest: Arc<Mutex<Option<DeepLinkTarget>>>,
}

impl DeepLinkInbox {
    pub(crate) fn latest(&self) -> Option<DeepLinkTarget> {
        self.latest.lock().ok().and_then(|value| value.clone())
    }

    pub(crate) fn accept(&self, raw: &str) -> Result<DeepLinkTarget, DeepLinkFailure> {
        let target = parse_deep_link(raw)?;
        let mut latest = self
            .latest
            .lock()
            .map_err(|_| DeepLinkFailure::new("desktop.deep_link.state_unavailable"))?;
        *latest = Some(target.clone());
        Ok(target)
    }
}

pub(crate) fn deliver_deep_links(app: &AppHandle, urls: impl IntoIterator<Item = Url>) {
    let inbox = app.state::<DeepLinkInbox>();
    for url in urls {
        let Ok(target) = inbox.accept(url.as_str()) else {
            continue;
        };
        let _ = app.emit(DEEP_LINK_EVENT, target);
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn parse_deep_link(raw: &str) -> Result<DeepLinkTarget, DeepLinkFailure> {
    let url = Url::parse(raw).map_err(|_| DeepLinkFailure::invalid())?;
    if url.scheme() != "agent-room"
        || url.username() != ""
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(DeepLinkFailure::invalid());
    }
    let host = url.host_str().ok_or_else(DeepLinkFailure::invalid)?;
    let segments = url
        .path_segments()
        .ok_or_else(DeepLinkFailure::invalid)?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.iter().any(|segment| !valid_identifier(segment)) {
        return Err(DeepLinkFailure::invalid());
    }
    match (host, segments.as_slice()) {
        ("lobby", [catalog_id]) => Ok(DeepLinkTarget {
            kind: DeepLinkKind::Lobby,
            route: format!("/lobby/{catalog_id}"),
        }),
        ("lobby", [catalog_id, "instance", room_id]) => Ok(DeepLinkTarget {
            kind: DeepLinkKind::Lobby,
            route: format!("/lobby/{catalog_id}/instance/{room_id}"),
        }),
        _ => Err(DeepLinkFailure::invalid()),
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'!')
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeepLinkFailure;

impl DeepLinkFailure {
    const fn new(_code: &'static str) -> Self {
        Self
    }

    const fn invalid() -> Self {
        Self::new("desktop.deep_link.invalid")
    }
}

#[cfg(test)]
mod tests {
    use super::{DeepLinkKind, parse_deep_link};

    #[test]
    fn 深链只能映射到闭合的产品路由() {
        let lobby = parse_deep_link(
            "agent-room://lobby/0198b601-77a2-7f41-b4f4-940f291951b8/instance/!room:example.org",
        )
        .expect("大厅深链有效");
        assert_eq!(lobby.kind, DeepLinkKind::Lobby);
        assert_eq!(
            lobby.route,
            "/lobby/0198b601-77a2-7f41-b4f4-940f291951b8/instance/!room:example.org"
        );

        let invalid =
            parse_deep_link("https://evil.example/command").expect_err("外部 URL 不是深链");
        assert_eq!(invalid, super::DeepLinkFailure);
        assert!(parse_deep_link("agent-room://lobby/id?next=https://evil.example").is_err());
        assert!(parse_deep_link("agent-room://handoff/unimplemented").is_err());
    }
}
