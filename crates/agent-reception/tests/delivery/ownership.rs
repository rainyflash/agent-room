use super::*;
use agent_room_agent_client::BridgeToolFailure;

struct ControlledBridge {
    inner: Bridge,
    quiet: bool,
    fail_close: bool,
    lost_release: bool,
    releases: AtomicUsize,
    heartbeats: AtomicUsize,
}
impl ControlledBridge {
    fn new(inner: Bridge) -> Self {
        Self {
            inner,
            quiet: false,
            fail_close: false,
            lost_release: false,
            releases: AtomicUsize::new(0),
            heartbeats: AtomicUsize::new(0),
        }
    }
}
impl BridgeToolClient for ControlledBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        Box::pin(async move {
            let plain = match &method {
                IpcMethod::WithSession { method, .. } => method.as_ref(),
                method => method,
            };
            if matches!(plain, IpcMethod::ReadInbox(_)) && self.quiet {
                return Ok(IpcResponse::MessagePreviews {
                    previews: vec![],
                    next_cursor: None,
                });
            }
            if matches!(plain, IpcMethod::CloseHostSession(_)) && self.fail_close {
                return Err(unavailable());
            }
            if matches!(
                plain,
                IpcMethod::ReceptionControl(ReceptionRequest {
                    command: ReceptionCommand::Heartbeat,
                    ..
                })
            ) {
                self.heartbeats.fetch_add(1, Ordering::Relaxed);
            }
            let release = matches!(
                plain,
                IpcMethod::ReceptionControl(ReceptionRequest {
                    command: ReceptionCommand::Release,
                    ..
                })
            );
            let response = self.inner.invoke(method).await?;
            if release && self.releases.fetch_add(1, Ordering::Relaxed) == 0 && self.lost_release {
                return Err(unavailable());
            }
            Ok(response)
        })
    }
}
fn unavailable() -> BridgeToolFailure {
    BridgeToolFailure::new(
        "test.transport_unavailable",
        IpcErrorCategory::DependencyUnavailable,
        true,
        std::collections::BTreeMap::new(),
    )
}
struct DrainDuringTurn {
    bridge: Bridge,
    cancelled: AtomicUsize,
}
struct Cancelled<'a>(&'a AtomicUsize);
impl Drop for Cancelled<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}
impl HostRunner for DrainDuringTurn {
    fn resume<'a>(&'a self, _: HostDelivery<'a>) -> HostFuture<'a> {
        Box::pin(async move {
            let _cancelled = Cancelled(&self.cancelled);
            self.bridge
                .reception
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .status = ReceptionStatus::Draining;
            std::future::pending().await
        })
    }
}

#[tokio::test(start_paused = true)]
async fn remote_takeover_cancels_turn_then_releases_with_pending_identity() {
    let (root, binding, bridge) = setup();
    let host = DrainDuringTurn {
        bridge: bridge.clone(),
        cancelled: AtomicUsize::new(0),
    };
    run(
        ReceiverContext {
            mode: ReceiverMode::Listen,
            host: &host,
            backend: &bridge,
            data_root: root.path(),
            service: "test.receiver",
            emit: &|_| Ok(()),
        },
        &binding.host.task_id,
        std::future::pending(),
    )
    .await
    .unwrap();
    assert_eq!(host.cancelled.load(Ordering::Relaxed), 1);
    let state = ReceiverStore::inspect(root.path(), &binding.host.task_id)
        .unwrap()
        .unwrap();
    assert!(!state.enabled);
    assert!(state.execution.is_none());
    assert!(matches!(
        state.checkpoint,
        ReceptionCheckpoint::Pending { .. }
    ));
    let reception = bridge.reception.lock().unwrap();
    let record = reception.as_ref().unwrap();
    assert_eq!(record.status, ReceptionStatus::Idle);
    assert_eq!(
        record
            .progress
            .pending
            .as_ref()
            .unwrap()
            .submission_id
            .to_string(),
        state.last_delivery.unwrap().submission_id
    );
}

#[tokio::test(start_paused = true)]
async fn idle_monitor_never_starts_model_and_close_failure_keeps_execution() {
    for fail_close in [false, true] {
        let (root, binding, bridge) = setup();
        let host = Host {
            bridge: bridge.clone(),
            outcome: Reply::Missing,
            calls: AtomicUsize::new(0),
        };
        let transport = ControlledBridge {
            quiet: true,
            fail_close,
            ..ControlledBridge::new(bridge)
        };
        let result = run(
            ReceiverContext {
                mode: ReceiverMode::Listen,
                host: &host,
                backend: &transport,
                data_root: root.path(),
                service: "test.receiver",
                emit: &|_| Ok(()),
            },
            &binding.host.task_id,
            tokio::time::sleep(std::time::Duration::from_secs(61)),
        )
        .await;
        assert_eq!(host.calls.load(Ordering::Relaxed), 0);
        assert!(transport.heartbeats.load(Ordering::Relaxed) >= 11);
        let state = ReceiverStore::inspect(root.path(), &binding.host.task_id)
            .unwrap()
            .unwrap();
        assert_eq!(state.execution.is_some(), fail_close);
        if fail_close {
            let error = result.unwrap_err();
            assert_eq!(error.code, "receiver.release_unconfirmed");
            assert_eq!(
                error.details.get("closeError").unwrap(),
                "test.transport_unavailable"
            );
            assert_eq!(transport.releases.load(Ordering::Relaxed), 0);
        } else {
            result.unwrap();
        }
    }
}

#[tokio::test(start_paused = true)]
async fn lost_release_response_resumes_the_same_release_without_another_turn() {
    let (root, binding, bridge) = setup();
    let host = Host {
        bridge: bridge.clone(),
        outcome: Reply::Valid,
        calls: AtomicUsize::new(0),
    };
    let transport = ControlledBridge {
        lost_release: true,
        ..ControlledBridge::new(bridge)
    };
    assert!(
        receive(
            root.path(),
            &binding,
            &transport,
            &host,
            ReceiverMode::Listen
        )
        .await
        .is_err()
    );
    let before = ReceiverStore::inspect(root.path(), &binding.host.task_id)
        .unwrap()
        .unwrap();
    assert!(before.execution.as_ref().unwrap().releasing);
    run(
        ReceiverContext {
            mode: ReceiverMode::Listen,
            host: &host,
            backend: &transport,
            data_root: root.path(),
            service: "test.receiver",
            emit: &|_| Ok(()),
        },
        &binding.host.task_id,
        async {},
    )
    .await
    .unwrap();
    let after = ReceiverStore::inspect(root.path(), &binding.host.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(before.checkpoint, after.checkpoint);
    assert!(after.execution.is_none());
    assert_eq!(host.calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.releases.load(Ordering::Relaxed), 2);
}
