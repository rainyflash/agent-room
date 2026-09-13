use agent_room_bridge_ipc::{
    IpcHostRoomTarget, IpcHostSessionDiagnostics, IpcHostSessionState, IpcHostSessionSummary,
    IpcReceptionHost, IpcReceptionOffer, IpcRegisterReceptionRequest,
};

#[test]
fn 原生诊断响应与前端消费的同一份契约保持一致() {
    let mut entry = IpcHostSessionDiagnostics {
        session: IpcHostSessionSummary {
            session_id: "0198b601-77a1-7bb8-83eb-a8fe68c97e44".into(),
            state: IpcHostSessionState::Ready,
            agent_id: Some("0198b601-77a2-7bb8-83eb-a8fe68c97e44".into()),
            error_code: None,
        },
        display_name: "Contract Agent".into(),
        room_id: Some("!public:matrix.test".into()),
        requested_room: Some(IpcHostRoomTarget {
            catalog_id: "0198b601-77a3-7bb8-83eb-a8fe68c97e44".into(),
            room_id: "!public:matrix.test".into(),
        }),
        session_key: Some("0198b601-77a4-7bb8-83eb-a8fe68c97e44".into()),
        reception_offer: Some(IpcReceptionOffer {
            room_catalog_id: Some("0198b601-77a3-7bb8-83eb-a8fe68c97e44".into()),
            instance_id: "0198b601-77a5-7bb8-83eb-a8fe68c97e44".into(),
            task: IpcRegisterReceptionRequest {
                host_type: IpcReceptionHost::ClaudeCode,
                task_id: "0198b601-77a6-7bb8-83eb-a8fe68c97e44".into(),
                workspace: "/project".into(),
            },
            room_id: "!public:matrix.test".into(),
        }),
        last_inbox_read_ago_ms: Some(1),
        last_message_received_ago_ms: Some(2),
        last_message_sent_ago_ms: Some(3),
    };
    let ready = entry.clone();
    entry.session.state = IpcHostSessionState::Closed;
    entry.session.agent_id = None;
    entry.room_id = None;
    entry.requested_room = None;
    entry.session_key = None;
    entry.reception_offer = None;
    entry.last_inbox_read_ago_ms = None;
    entry.last_message_received_ago_ms = None;
    entry.last_message_sent_ago_ms = None;
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/host-session-diagnostics.json")).unwrap();
    assert_eq!(serde_json::to_value([ready, entry]).unwrap(), expected);
}
