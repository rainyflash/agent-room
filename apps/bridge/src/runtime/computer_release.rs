//! 换账号时把这台电脑移出原来的账户（`specs/sign-out/design.md` 第 4 步）。
//!
//! 桌面端写一行 `release`，Bridge 拿这台电脑的设备凭据请控制面移出它。没成就报
//! `computer_release_failed`，照常运行、等桌面端再写；成了就有序退出，清掉本机这份设备凭据，最后报
//! `computer_released`。数据目录里属于原账户的东西由桌面端在 Bridge 退出以后收走。

use agent_room_application::ports::PortFuture;
use agent_room_bridge::control_plane::computer_release::{
    ComputerReleaseFailure, ComputerReleaseOutcome, ReqwestComputerRelease,
};

use super::supervisor_input::SupervisorInput;

pub(super) trait ReleaseThisComputer: Send + Sync {
    /// 没成时给稳定的错误码。
    fn release(&self) -> PortFuture<'_, Result<ComputerReleaseOutcome, &'static str>>;
}

impl ReleaseThisComputer for ReqwestComputerRelease {
    fn release(&self) -> PortFuture<'_, Result<ComputerReleaseOutcome, &'static str>> {
        Box::pin(async move {
            Self::release(self)
                .await
                .map_err(ComputerReleaseFailure::code)
        })
    }
}

/// 等桌面端写 `release` 并移出这台电脑；没成就报失败、接着等下一次。只在移出去了以后返回。
pub(super) async fn release_when_requested(
    input: &mut SupervisorInput,
    computer: &dyn ReleaseThisComputer,
    announce_failed: impl Fn(&'static str),
) -> ComputerReleaseOutcome {
    loop {
        input.next_release().await;
        match computer.release().await {
            Ok(outcome) => {
                tracing::info!(?outcome, "这台电脑已移出原来的账户，Bridge 接着有序退出");
                return outcome;
            }
            Err(code) => {
                tracing::warn!(
                    error_code = code,
                    "没能把这台电脑移出原来的账户，等桌面端再试"
                );
                announce_failed(code);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Write as _},
        sync::Mutex,
        time::Duration,
    };

    use agent_room_bridge::control_plane::computer_release::RELEASE_UNAVAILABLE;

    use super::*;

    /// 按顺序回答每一次移出。
    struct 脚本移出 {
        script: Mutex<Vec<Result<ComputerReleaseOutcome, &'static str>>>,
    }

    impl ReleaseThisComputer for 脚本移出 {
        fn release(&self) -> PortFuture<'_, Result<ComputerReleaseOutcome, &'static str>> {
            let next = self.script.lock().expect("脚本锁未中毒").remove(0);
            Box::pin(async move { next })
        }
    }

    #[tokio::test]
    async fn 没移出去就报失败_接着等桌面端再写_release() {
        let (reader, mut writer) = io::pipe().expect("可创建管道");
        let mut input = SupervisorInput::watch(reader);
        let computer = 脚本移出 {
            script: Mutex::new(vec![
                Err(RELEASE_UNAVAILABLE),
                Ok(ComputerReleaseOutcome::Released),
            ]),
        };
        let failures = Mutex::new(Vec::new());
        writer.write_all(b"release\n").expect("可写入管道");

        let release = release_when_requested(&mut input, &computer, |code| {
            failures.lock().expect("失败记录锁未中毒").push(code);
        });
        tokio::pin!(release);
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut release)
                .await
                .is_err(),
            "第一次没成，要等桌面端再写一次"
        );
        assert_eq!(
            *failures.lock().expect("失败记录锁未中毒"),
            [RELEASE_UNAVAILABLE]
        );

        writer.write_all(b"release\n").expect("可写入管道");
        let outcome = tokio::time::timeout(Duration::from_secs(5), release)
            .await
            .expect("第二次移出去了");
        assert_eq!(outcome, ComputerReleaseOutcome::Released);
        assert_eq!(failures.lock().expect("失败记录锁未中毒").len(), 1);
    }
}
