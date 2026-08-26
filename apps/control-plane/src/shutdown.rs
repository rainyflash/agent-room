use std::future::pending;

pub(crate) async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let terminate = async {
            if let Ok(mut stream) = signal(SignalKind::terminate()) {
                stream.recv().await;
            } else {
                tracing::error!(
                    code = "shutdown.sigterm_listener_failed",
                    "无法监听 SIGTERM"
                );
                pending::<()>().await;
            }
        };

        tokio::select! {
            () = wait_for_ctrl_c() => {},
            () = terminate => {},
        }
    }

    #[cfg(not(unix))]
    wait_for_ctrl_c().await;

    tracing::info!("收到关闭信号，开始优雅关闭");
}

async fn wait_for_ctrl_c() {
    if tokio::signal::ctrl_c().await.is_err() {
        tracing::error!(code = "shutdown.ctrl_c_listener_failed", "无法监听 Ctrl+C");
        pending::<()>().await;
    }
}
