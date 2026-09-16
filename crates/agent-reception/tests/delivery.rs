use agent_room_agent_client::{BridgeToolClient, BridgeToolFuture, reception::ReceptionCheckpoint};
use agent_room_agent_reception::*;
use agent_room_bridge_ipc::*;
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[path = "delivery/ownership.rs"]
mod ownership;

#[derive(Clone)]
struct Bridge {
    identity: String,
    source: IpcMessagePreviewSummary,
    replies: Arc<Mutex<Vec<IpcMessagePreviewSummary>>>,
    reception: Arc<Mutex<Option<ReceptionRecord>>>,
    outcome: Arc<Mutex<Reply>>,
    sent: Arc<Mutex<Vec<IpcSendMessageRequest>>>,
}
impl Bridge {
    fn publish(&self, request: &IpcSendMessageRequest, outcome: Reply) {
        if matches!(outcome, Reply::Missing) {
            return;
        }
        let mut message = self.source.clone();
        message.message_id = request.submission_id.clone().unwrap();
        message.event_id = "$reply".into();
        message.reply_to_message_id = Some(if matches!(outcome, Reply::WrongRelation) {
            uuid::Uuid::now_v7().to_string()
        } else {
            self.source.message_id.clone()
        });
        message.actor = IpcActorSummary::Agent {
            agent: IpcAgentSummary {
                agent_id: if matches!(outcome, Reply::WrongAgent) {
                    uuid::Uuid::now_v7().to_string()
                } else {
                    self.identity.clone()
                },
                display_name: "Receiver".into(),
                matrix_user_id: "@agent:test".into(),
                avatar_url: None,
            },
            instance_id: self.identity.clone(),
            provenance: IpcMessageProvenance::AutonomousAgent,
        };
        self.replies.lock().unwrap().push(message);
    }

    fn control(&self, request: ReceptionRequest) -> IpcResponse {
        let mut reception = self.reception.lock().unwrap();
        match request.command {
            ReceptionCommand::Claim {
                session_key,
                display_name,
                initial,
            } => {
                let previous = reception.take();
                assert!(
                    previous
                        .as_ref()
                        .is_none_or(|record| record.status == ReceptionStatus::Idle
                            || record.run_id == request.run_id)
                );
                *reception = Some(ReceptionRecord {
                    agent_id: request.agent_id,
                    instance_id: request.instance_id,
                    catalog_id: request.catalog_id,
                    room_id: request.room_id,
                    session_key,
                    display_name,
                    device_id: request.instance_id,
                    device_label: "Test computer".into(),
                    run_id: request.run_id,
                    status: ReceptionStatus::Active,
                    next_device_id: None,
                    last_seen_unix_ms: 1,
                    revision: previous.as_ref().map_or(0, |record| record.revision),
                    progress: previous.map_or(initial, |record| record.progress),
                });
            }
            ReceptionCommand::Save { revision, progress } => {
                let record = reception.as_mut().unwrap();
                assert_eq!(record.run_id, request.run_id);
                assert_eq!(record.revision, revision);
                record.progress = progress;
                record.revision += 1;
            }
            ReceptionCommand::Release => {
                reception.as_mut().unwrap().status = ReceptionStatus::Idle;
            }
            ReceptionCommand::Heartbeat => {}
        }
        IpcResponse::Reception {
            record: reception.as_ref().unwrap().clone(),
        }
    }
}
impl BridgeToolClient for Bridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        Box::pin(async move {
            let method = match method {
                IpcMethod::WithSession { method, .. } => *method,
                method => method,
            };
            Ok(match method {
                IpcMethod::ReceptionControl(request) => self.control(request),
                IpcMethod::SendReceptionMessage { run_id, request } => {
                    assert_eq!(
                        run_id,
                        self.reception.lock().unwrap().as_ref().unwrap().run_id
                    );
                    assert_eq!(request.room_id, self.source.room_id);
                    assert_eq!(
                        request.reply_to_message_id.as_deref(),
                        Some(self.source.message_id.as_str())
                    );
                    assert_eq!(request.provenance, IpcMessageProvenance::AutonomousAgent);
                    assert!(request.automation_grant_id.is_some() && request.chat);
                    self.sent.lock().unwrap().push(request.clone());
                    let outcome = *self.outcome.lock().unwrap();
                    self.publish(&request, outcome);
                    if matches!(outcome, Reply::ValidButSendFails) {
                        return Err(agent_room_agent_client::BridgeToolFailure::new(
                            "test.send_response_lost",
                            IpcErrorCategory::DependencyUnavailable,
                            true,
                            std::collections::BTreeMap::new(),
                        ));
                    }
                    IpcResponse::SentMessage {
                        message: IpcSentMessage {
                            submission_id: request.submission_id.unwrap(),
                            state: IpcSubmissionState::Submitted,
                            event_id: Some("$reply".into()),
                        },
                    }
                }
                IpcMethod::OpenHostSession(_) => IpcResponse::HostSession {
                    session: IpcHostSessionSummary {
                        session_id: self.identity.clone(),
                        state: IpcHostSessionState::Ready,
                        agent_id: Some(self.identity.clone()),
                        error_code: None,
                    },
                },
                IpcMethod::CloseHostSession(_) => IpcResponse::HostSession {
                    session: IpcHostSessionSummary {
                        session_id: self.identity.clone(),
                        state: IpcHostSessionState::Closed,
                        agent_id: Some(self.identity.clone()),
                        error_code: None,
                    },
                },
                IpcMethod::GetSelf => IpcResponse::SelfSummary {
                    summary: IpcSelfSummary {
                        agent: IpcAgentSummary {
                            agent_id: self.identity.clone(),
                            display_name: "Receiver".into(),
                            matrix_user_id: "@agent:test".into(),
                            avatar_url: None,
                        },
                        room_catalog_id: Some(self.identity.clone()),
                        instance_id: self.identity.clone(),
                        matrix_device_id: "DEVICE".into(),
                        room_id: "!room:test".into(),
                        connection_state: IpcBridgeState::Ready,
                        granted_capabilities: vec![],
                    },
                },
                IpcMethod::ReadInbox(request) | IpcMethod::WaitInbox(request) => {
                    let previews = if request.after_event_id.is_none() {
                        vec![self.source.clone()]
                    } else if request.after_event_id.as_deref() == Some("$input") {
                        self.replies.lock().unwrap().clone()
                    } else {
                        vec![]
                    };
                    IpcResponse::MessagePreviews {
                        previews,
                        next_cursor: None,
                    }
                }
                method => panic!("unexpected {}", method.name()),
            })
        })
    }
}

#[derive(Clone, Copy)]
enum Reply {
    Valid,
    ValidButSendFails,
    Missing,
    WrongRelation,
    WrongAgent,
    HostFails,
}
struct Host {
    bridge: Bridge,
    outcome: Reply,
    calls: AtomicUsize,
}
impl HostRunner for Host {
    fn resume<'a>(&'a self, delivery: HostDelivery<'a>) -> HostFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let state = ReceiverStore::inspect(delivery.data_root, &delivery.binding.task_id)
                .unwrap()
                .unwrap();
            assert!(matches!(
                state.checkpoint,
                ReceptionCheckpoint::Pending { .. }
            ));
            assert_eq!(
                state.last_delivery.unwrap().submission_id,
                delivery.submission_id
            );
            *self.bridge.outcome.lock().unwrap() = self.outcome;
            if matches!(self.outcome, Reply::HostFails) {
                return Err(ReceptionFailure::local("receiver.host_failed"));
            }
            HostReply::new("reply content from the host".into())
        })
    }
}
fn setup() -> (tempfile::TempDir, ReceiverBinding, Bridge) {
    let root = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::now_v7().to_string();
    let binding: ReceiverBinding = serde_json::from_value(json!({
        "session":{"sessionKey":id,"displayName":"Receiver"}, "policy":{"roomId":"!room:test","allowedPrincipalId":id},
        "automationGrantId":id, "host":{"taskId":id,"executable":std::env::current_exe().unwrap(),"mcpExecutable":std::env::current_exe().unwrap(),"workspace":root.path()},
        "start":{"mode":"beginning"}
    })).unwrap();
    let source = serde_json::from_value(json!({
        "conversation":{"text":"hello","mentions":["@agent:test"]},"replyToMessageId":null,
        "messageId":id,"eventId":"$input","roomId":"!room:test",
        "actor":{"kind":"human","principalId":id,"displayName":"Owner","matrixUserId":"@owner:test","avatarUrl":null},
        "createdAtUnixMs":1,"title":"hello","summary":"hello","content":{"contentId":id,"digestSha256":"0".repeat(64),"mediaType":"text/plain","sizeBytes":5},
        "language":null,"sensitivity":"normal","riskFlags":[]
    })).unwrap();
    ReceiverStore::open(root.path(), &id)
        .unwrap()
        .configure(binding.clone(), "test.receiver")
        .unwrap();
    (
        root,
        binding,
        Bridge {
            identity: id,
            source,
            replies: Arc::default(),
            reception: Arc::default(),
            outcome: Arc::new(Mutex::new(Reply::Valid)),
            sent: Arc::default(),
        },
    )
}
async fn receive(
    root: &std::path::Path,
    binding: &ReceiverBinding,
    bridge: &dyn BridgeToolClient,
    host: &Host,
    mode: ReceiverMode,
) -> ReceptionResult<()> {
    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    let emit = |event| {
        if matches!(event, ReceiverEvent::Delivery {record} if record.stage == DeliveryStage::Replied)
        {
            stop.send_replace(true);
        }
        Ok(())
    };
    run(
        ReceiverContext {
            backend: bridge,
            data_root: root,
            service: "test.receiver",
            emit: &emit,
            host,
            mode,
        },
        &binding.host.task_id,
        async {
            let _ = stopped.wait_for(|done| *done).await;
        },
    )
    .await
}

#[tokio::test(start_paused = true)]
async fn only_a_matching_room_reply_advances_the_cursor() {
    for outcome in [
        Reply::Missing,
        Reply::WrongRelation,
        Reply::WrongAgent,
        Reply::Valid,
        Reply::ValidButSendFails,
    ] {
        let (root, binding, bridge) = setup();
        let host = Host {
            bridge: bridge.clone(),
            outcome,
            calls: AtomicUsize::new(0),
        };
        let result = receive(root.path(), &binding, &bridge, &host, ReceiverMode::Listen).await;
        let saved = ReceiverStore::inspect(root.path(), &binding.host.task_id)
            .unwrap()
            .unwrap();
        assert_eq!(host.calls.load(Ordering::Relaxed), 1);
        if matches!(outcome, Reply::Valid | Reply::ValidButSendFails) {
            result.unwrap();
            assert_eq!(saved.checkpoint.cursor(), Some("$input"));
            assert_eq!(
                saved.last_delivery.unwrap().reply_event_id.as_deref(),
                Some("$reply")
            );
        } else {
            assert_eq!(result.unwrap_err().code, "receiver.reply_unconfirmed");
            assert!(matches!(
                saved.checkpoint,
                ReceptionCheckpoint::Pending { .. }
            ));
            assert_eq!(
                saved.last_delivery.unwrap().stage,
                DeliveryStage::NeedsReview
            );
        }
    }
}

struct InterruptedBridge {
    inner: Bridge,
    failures: AtomicUsize,
}

struct DelayedBridge {
    inner: Bridge,
    ready_at: tokio::time::Instant,
    opens: AtomicUsize,
}
impl BridgeToolClient for DelayedBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        if matches!(&method, IpcMethod::OpenHostSession(_)) {
            self.opens.fetch_add(1, Ordering::Relaxed);
        }
        if matches!(&method, IpcMethod::WithSession { method, .. } if matches!(method.as_ref(), IpcMethod::GetSelf))
            && tokio::time::Instant::now() < self.ready_at
        {
            return Box::pin(async {
                Err(agent_room_agent_client::BridgeToolFailure::new(
                    "bridge.agent_runtime_unavailable",
                    IpcErrorCategory::DependencyUnavailable,
                    true,
                    std::collections::BTreeMap::new(),
                ))
            });
        }
        self.inner.invoke(method)
    }
}

#[tokio::test(start_paused = true)]
async fn prolonged_network_outage_recovers_without_manual_restart() {
    let (root, binding, bridge) = setup();
    let host = Host {
        bridge: bridge.clone(),
        outcome: Reply::Valid,
        calls: AtomicUsize::new(0),
    };
    let backend = DelayedBridge {
        inner: bridge,
        ready_at: tokio::time::Instant::now() + std::time::Duration::from_mins(3),
        opens: AtomicUsize::new(0),
    };
    tokio::time::timeout(
        std::time::Duration::from_mins(5),
        receive(root.path(), &binding, &backend, &host, ReceiverMode::Listen),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(backend.opens.load(Ordering::Relaxed) > 1);
    assert_eq!(host.calls.load(Ordering::Relaxed), 1);
    let saved = ReceiverStore::inspect(root.path(), &binding.host.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.checkpoint.cursor(), Some("$input"));
    assert_eq!(saved.last_delivery.unwrap().stage, DeliveryStage::Replied);
}
impl BridgeToolClient for InterruptedBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        let verifying = matches!(&method, IpcMethod::WithSession { method, .. }
            if matches!(method.as_ref(), IpcMethod::ReadInbox(request) | IpcMethod::WaitInbox(request) if request.after_event_id.as_deref() == Some("$input")));
        if verifying && self.failures.fetch_add(1, Ordering::Relaxed) == 0 {
            return Box::pin(async {
                Err(agent_room_agent_client::BridgeToolFailure::new(
                    "test.network_unavailable",
                    IpcErrorCategory::DependencyUnavailable,
                    true,
                    std::collections::BTreeMap::new(),
                ))
            });
        }
        self.inner.invoke(method)
    }
}

#[tokio::test(start_paused = true)]
async fn network_failure_after_sending_only_reconciles_without_a_second_host_turn() {
    let (root, binding, bridge) = setup();
    let host = Host {
        bridge: bridge.clone(),
        outcome: Reply::Valid,
        calls: AtomicUsize::new(0),
    };
    let backend = InterruptedBridge {
        inner: bridge,
        failures: AtomicUsize::new(0),
    };
    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    let reconnects = AtomicUsize::new(0);
    let emit = |event| {
        match event {
            ReceiverEvent::Reconnecting { .. } => {
                reconnects.fetch_add(1, Ordering::Relaxed);
            }
            ReceiverEvent::Delivery { record } if record.stage == DeliveryStage::Replied => {
                stop.send_replace(true);
            }
            _ => {}
        }
        Ok(())
    };
    run(
        ReceiverContext {
            mode: ReceiverMode::Listen,
            host: &host,
            backend: &backend,
            data_root: root.path(),
            service: "test.receiver",
            emit: &emit,
        },
        &binding.host.task_id,
        async {
            let _ = stopped.wait_for(|stop| *stop).await;
        },
    )
    .await
    .unwrap();
    assert_eq!(reconnects.load(Ordering::Relaxed), 1);
    assert_eq!(host.calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        ReceiverStore::inspect(root.path(), &binding.host.task_id)
            .unwrap()
            .unwrap()
            .last_delivery
            .unwrap()
            .stage,
        DeliveryStage::Replied
    );
}

struct WaitingHost(tokio::sync::Notify);
impl HostRunner for WaitingHost {
    fn resume<'a>(&'a self, _: HostDelivery<'a>) -> HostFuture<'a> {
        Box::pin(async move {
            self.0.notify_one();
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn pause_during_host_execution_keeps_pending_and_releases_the_receiver_lock() {
    let (root, binding, bridge) = setup();
    let host = WaitingHost(tokio::sync::Notify::new());
    let emit = |_| Ok(());
    run(
        ReceiverContext {
            mode: ReceiverMode::Listen,
            host: &host,
            backend: &bridge,
            data_root: root.path(),
            service: "test.receiver",
            emit: &emit,
        },
        &binding.host.task_id,
        host.0.notified(),
    )
    .await
    .unwrap();
    let store = ReceiverStore::open(root.path(), &binding.host.task_id).unwrap();
    let saved = store.load().unwrap().unwrap();
    assert!(matches!(
        saved.checkpoint,
        ReceptionCheckpoint::Pending { .. }
    ));
    assert_eq!(saved.last_delivery.unwrap().stage, DeliveryStage::Running);
}

#[tokio::test(start_paused = true)]
async fn renewal_and_explicit_retry_preserve_identity_and_submission_id() {
    let (root, binding, bridge) = setup();
    let failed = Host {
        bridge: bridge.clone(),
        outcome: Reply::Missing,
        calls: AtomicUsize::new(0),
    };
    receive(
        root.path(),
        &binding,
        &bridge,
        &failed,
        ReceiverMode::Listen,
    )
    .await
    .unwrap_err();
    let store = ReceiverStore::open(root.path(), &binding.host.task_id).unwrap();
    let before = store.load().unwrap().unwrap();
    let mut renewed = binding.clone();
    renewed.automation_grant_id = uuid::Uuid::now_v7().to_string();
    let after = store.configure(renewed.clone(), "test.receiver").unwrap();
    assert_eq!(before.checkpoint, after.checkpoint);
    assert_eq!(before.agent_id, after.agent_id);
    assert_eq!(
        before.last_delivery.as_ref().unwrap().submission_id,
        after.last_delivery.unwrap().submission_id
    );
    let mut moved = renewed.clone();
    moved.policy.room_id = "!other:test".into();
    assert_eq!(
        store.configure(moved, "test.receiver").unwrap_err().code,
        "receiver.identity_change_forbidden"
    );
    store.resolve("$input", Resolution::Retry).unwrap();
    drop(store);
    let replying = Host {
        bridge: bridge.clone(),
        outcome: Reply::Valid,
        calls: AtomicUsize::new(0),
    };
    receive(
        root.path(),
        &renewed,
        &bridge,
        &replying,
        ReceiverMode::Listen,
    )
    .await
    .unwrap();
    let after = ReceiverStore::inspect(root.path(), &binding.host.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        before.last_delivery.unwrap().submission_id,
        after.last_delivery.unwrap().submission_id
    );
}

#[test]
fn lock_paths_and_legacy_state_remain_safe() {
    let (root, binding, _) = setup();
    let locked = ReceiverStore::open(root.path(), &binding.host.task_id).unwrap();
    assert!(ReceiverStore::open(root.path(), &binding.host.task_id).is_err());
    assert!(ReceiverStore::open(root.path(), "../outside").is_err());
    let mut value = serde_json::to_value(locked.load().unwrap().unwrap()).unwrap();
    for field in ["lastDelivery", "enabled", "roomCatalogId", "instanceId"] {
        value.as_object_mut().unwrap().remove(field);
    }
    value["agentId"] = json!(binding.host.task_id);
    let legacy: ReceiverState = serde_json::from_value(value).unwrap();
    assert!(!legacy.enabled);
    assert!(legacy.last_delivery.is_none());
    drop(locked);
    ReceiverStore::open(root.path(), &binding.host.task_id).unwrap();
}

#[tokio::test(start_paused = true)]
async fn recovered_receipt_never_starts_another_host_turn() {
    let (root, binding, bridge) = setup();
    let missing = Host {
        bridge: bridge.clone(),
        outcome: Reply::Missing,
        calls: AtomicUsize::new(0),
    };
    receive(
        root.path(),
        &binding,
        &bridge,
        &missing,
        ReceiverMode::Listen,
    )
    .await
    .unwrap_err();
    let saved = ReceiverStore::inspect(root.path(), &binding.host.task_id)
        .unwrap()
        .unwrap();
    let submission_id = saved.last_delivery.unwrap().submission_id;
    let request = bridge.sent.lock().unwrap()[0].clone();
    assert_eq!(
        request.submission_id.as_deref(),
        Some(submission_id.as_str())
    );
    // The delayed original publication becomes visible; no model is run to manufacture it.
    bridge.publish(&request, Reply::Valid);
    // Receipt recovery must work even after the host executable was removed or upgraded.
    let store = ReceiverStore::open(root.path(), &binding.host.task_id).unwrap();
    let mut state = store.load().unwrap().unwrap();
    state.binding.host.executable = root.path().join("uninstalled-host.exe");
    store.save(&state).unwrap();
    drop(store);
    receive(
        root.path(),
        &binding,
        &bridge,
        &missing,
        ReceiverMode::VerifyReceipt,
    )
    .await
    .unwrap();
    assert_eq!(missing.calls.load(Ordering::Relaxed), 1);
    let saved = ReceiverStore::inspect(root.path(), &binding.host.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.last_delivery.unwrap().stage, DeliveryStage::Replied);
}

#[tokio::test(start_paused = true)]
async fn model_content_is_sent_under_the_exact_bound_authority() {
    let (root, binding, bridge) = setup();
    let host = Host {
        bridge: bridge.clone(),
        outcome: Reply::Valid,
        calls: AtomicUsize::new(0),
    };
    receive(root.path(), &binding, &bridge, &host, ReceiverMode::Listen)
        .await
        .unwrap();
    let sent = bridge.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].automation_grant_id.as_deref(),
        Some(binding.automation_grant_id.as_str())
    );
    assert_eq!(sent[0].body, "reply content from the host");
    assert_eq!(sent[0].provenance, IpcMessageProvenance::AutonomousAgent);
    assert_eq!(sent[0].room_id, binding.policy.room_id);
    assert_eq!(
        sent[0].reply_to_message_id.as_deref(),
        Some(bridge.source.message_id.as_str())
    );
    assert!(sent[0].mentions.is_empty());
}

#[tokio::test(start_paused = true)]
async fn failed_host_does_not_send_or_advance_the_pending_delivery() {
    let (root, binding, bridge) = setup();
    let host = Host {
        bridge: bridge.clone(),
        outcome: Reply::HostFails,
        calls: AtomicUsize::new(0),
    };
    let error = receive(root.path(), &binding, &bridge, &host, ReceiverMode::Listen)
        .await
        .unwrap_err();
    assert_eq!(error.code, "receiver.host_failed");
    assert!(bridge.sent.lock().unwrap().is_empty());
    let state = ReceiverStore::inspect(root.path(), &binding.host.task_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        state.checkpoint,
        ReceptionCheckpoint::Pending { .. }
    ));
}
