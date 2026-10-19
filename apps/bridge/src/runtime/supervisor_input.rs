//! 桌面端经标准输入发给 Bridge 的行（桌面端 `bridge_supervisor.rs`）。
//!
//! 标准输入是桌面持有的管道，平时不写：桌面退出前写一行 `exit` 请 Bridge 先退，管道关了说明桌面已经
//! 不在（包括被强制结束）。桌面端登录一次就好（`specs/desktop-sign-in/design.md`）以后又多两种行：
//! `device-grant <电脑授权>` 交来电脑授权，`device-code` 请 Bridge 改走设备码。换账号
//! （`specs/sign-out/design.md` 第 4 步）又多一行 `release`：把这台电脑移出原来的账户。同一个线程按行读、
//! 分开处理，不认识的行跳过。

use std::{future::Future, io};

use agent_room_application::ports::SecretValue;
use agent_room_bridge_core::authorization::{AuthorizedBridgeDevice, BridgeAuthorizationFailure};
use tokio::sync::{mpsc, watch};

use super::{BridgeRuntimeError, FirstDeviceAuthorization};

/// 桌面端要退出时写这一行（桌面端 `bridge_supervisor.rs` 的 `BRIDGE_EXIT_REQUEST`）。
const EXIT_REQUEST: &str = "exit";
const DEVICE_GRANT_PREFIX: &str = "device-grant ";
const DEVICE_CODE_REQUEST: &str = "device-code";
const RELEASE_REQUEST: &str = "release";

/// 监督 Bridge 的桌面为什么放手。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SupervisorRelease {
    /// 桌面端正常退出前请 Bridge 先退，自己等着。
    ExitRequested,
    /// 管道关了：桌面已经不在（包括被强制结束），或者它是不会写这一行的旧版。
    InputClosed,
}

/// 桌面端交来的下一步。
#[derive(Debug)]
pub(super) enum SupervisorCommand {
    DeviceGrant(SecretValue),
    DeviceCode,
    /// 把这台电脑移出原来的账户。
    Release,
}

enum SupervisorLine {
    Exit,
    Command(SupervisorCommand),
}

pub(super) struct SupervisorInput {
    commands: mpsc::UnboundedReceiver<SupervisorCommand>,
    released: watch::Receiver<Option<SupervisorRelease>>,
}

impl SupervisorInput {
    /// 在独立线程上按行读桌面的管道。阻塞读不放进 `spawn_blocking`：运行时关闭时会等它返回，
    /// 而桌面还在时这次读取不会返回。
    pub(super) fn watch(input: impl io::Read + Send + 'static) -> Self {
        let (command_sender, commands) = mpsc::unbounded_channel();
        let (release_sender, released) = watch::channel(None);
        // 起不了线程时两头都关着：等下一步立刻落空、改走设备码，退出只听操作系统的信号。
        let _ = std::thread::Builder::new()
            .name("agent-room-supervisor-watch".to_owned())
            .spawn(move || {
                let mut release = SupervisorRelease::InputClosed;
                for line in io::BufRead::lines(io::BufReader::new(input)) {
                    let Ok(line) = line else { break };
                    match parse(&line) {
                        Some(SupervisorLine::Exit) => {
                            release = SupervisorRelease::ExitRequested;
                            break;
                        }
                        Some(SupervisorLine::Command(command)) => {
                            let _ = command_sender.send(command);
                        }
                        None => {}
                    }
                }
                // 先记下为什么放手，再关掉下一步的通道：等下一步落空时看得到是不是桌面要退出。
                release_sender.send_replace(Some(release));
                drop(command_sender);
            });
        Self { commands, released }
    }

    /// 桌面请 Bridge 退出、或者管道读到结束和读取失败时完成。读不了管道时一直不完成，
    /// 照旧只响应操作系统关闭信号。
    pub(super) fn released(&self) -> impl Future<Output = SupervisorRelease> + Send + 'static {
        let mut released = self.released.clone();
        async move {
            let release = released
                .wait_for(Option::is_some)
                .await
                .map(|release| *release);
            match release {
                Ok(release) => release.unwrap_or(SupervisorRelease::InputClosed),
                Err(_) => std::future::pending().await,
            }
        }
    }

    fn has_released(&self) -> bool {
        self.released.borrow().is_some()
    }

    async fn next_command(&mut self) -> Option<SupervisorCommand> {
        self.commands.recv().await
    }

    /// 丢掉就绪之前攒下的行。桌面端只在 Bridge 报就绪以后才写 `release`；设备码授权那会儿没人读，
    /// 写进来的 `release` 不能留到授权完，把刚授权的这台电脑又移出去。
    pub(super) fn discard_pending(&mut self) {
        while self.commands.try_recv().is_ok() {}
    }

    /// 等桌面端写下一行 `release`，别的行跳过。管道关了就一直不返回：退出由 [`Self::released`] 管。
    pub(super) async fn next_release(&mut self) {
        loop {
            match self.next_command().await {
                Some(SupervisorCommand::Release) => return,
                Some(SupervisorCommand::DeviceGrant(_) | SupervisorCommand::DeviceCode) => {}
                None => std::future::pending().await,
            }
        }
    }
}

fn parse(line: &str) -> Option<SupervisorLine> {
    let line = line.trim();
    if line == EXIT_REQUEST {
        return Some(SupervisorLine::Exit);
    }
    if line == DEVICE_CODE_REQUEST {
        return Some(SupervisorLine::Command(SupervisorCommand::DeviceCode));
    }
    if line == RELEASE_REQUEST {
        return Some(SupervisorLine::Command(SupervisorCommand::Release));
    }
    line.strip_prefix(DEVICE_GRANT_PREFIX)
        .and_then(|grant| SecretValue::new(grant.trim()).ok())
        .map(|grant| SupervisorLine::Command(SupervisorCommand::DeviceGrant(grant)))
}

/// 等桌面端交电脑授权的结果。
pub(super) enum GrantOutcome {
    Authorized(AuthorizedBridgeDevice),
    /// 桌面端要不到电脑授权，请 Bridge 改走设备码。
    DeviceCode,
    /// 还没授权完桌面端就要退出了。
    Released,
    /// 还没授权完桌面端就要换账号：服务器上没有这台电脑可移，直接退出。
    ReleaseRequested,
}

/// 桌面端托管、设了 `AGENT_ROOM_BRIDGE_DEVICE_AUTHORIZATION=supervisor` 时的首次授权：调用方先报
/// `device_grant_required`，这里等桌面端交来电脑授权、拿它注册这台电脑。没成就报
/// `device_grant_failed`，接着等下一行；桌面端写 `device-code` 就改走设备码。
pub(super) async fn authorize_with_supervisor_grant(
    authorization: &impl FirstDeviceAuthorization,
    input: &mut SupervisorInput,
    announce_failed: impl Fn(BridgeAuthorizationFailure) -> Result<(), BridgeRuntimeError>,
) -> Result<GrantOutcome, BridgeRuntimeError> {
    loop {
        let Some(command) = input.next_command().await else {
            return Ok(if input.has_released() {
                GrantOutcome::Released
            } else {
                GrantOutcome::DeviceCode
            });
        };
        let grant = match command {
            SupervisorCommand::DeviceCode => return Ok(GrantOutcome::DeviceCode),
            SupervisorCommand::Release => return Ok(GrantOutcome::ReleaseRequested),
            SupervisorCommand::DeviceGrant(grant) => grant,
        };
        match authorization.attempt_with_grant(grant).await {
            Ok(device) => return Ok(GrantOutcome::Authorized(device)),
            Err(failure) => {
                tracing::warn!(
                    operation = failure.operation(),
                    failure_kind = ?failure.kind(),
                    "拿电脑授权注册这台电脑没成，等桌面端下一步"
                );
                announce_failed(failure)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write as _,
        sync::{
            Mutex,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };

    use agent_room_application::ports::{OidcDeviceAuthorizationPromptSink, PortFuture};
    use agent_room_domain::{ids::DeviceId, time::UtcMillis};

    use super::*;

    #[tokio::test]
    async fn 监督管道关闭后才触发随桌面退出() {
        let (reader, writer) = io::pipe().expect("可创建管道");
        let released = SupervisorInput::watch(reader).released();
        tokio::pin!(released);

        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut released)
                .await
                .is_err(),
            "桌面还持有管道时 Bridge 不能退出"
        );
        drop(writer);
        let release = tokio::time::timeout(Duration::from_secs(5), released)
            .await
            .expect("桌面一退出，管道关闭就应触发退出");
        assert_eq!(release, SupervisorRelease::InputClosed);
    }

    #[tokio::test]
    async fn 桌面端写一行_exit_就请_bridge_有序退出_别的内容不算() {
        let (reader, mut writer) = io::pipe().expect("可创建管道");
        let released = SupervisorInput::watch(reader).released();
        tokio::pin!(released);

        writer.write_all(b"hello\n").expect("可写入管道");
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut released)
                .await
                .is_err(),
            "不认识的内容不能让 Bridge 退出"
        );
        // 桌面端还握着管道：只凭这一行就要退出，不等管道关闭。
        writer.write_all(b"exit\n").expect("可写入管道");
        let release = tokio::time::timeout(Duration::from_secs(5), released)
            .await
            .expect("读到 exit 就应触发退出");
        assert_eq!(release, SupervisorRelease::ExitRequested);
    }

    /// 按顺序回答每一次拿电脑授权注册，记下交来的授权。
    struct 脚本注册 {
        script: Mutex<Vec<Result<AuthorizedBridgeDevice, BridgeAuthorizationFailure>>>,
        grants: Mutex<Vec<String>>,
        device_code_attempts: AtomicU32,
    }

    impl 脚本注册 {
        fn new(
            script: impl IntoIterator<Item = Result<AuthorizedBridgeDevice, BridgeAuthorizationFailure>>,
        ) -> Self {
            let mut script: Vec<_> = script.into_iter().collect();
            script.reverse();
            Self {
                script: Mutex::new(script),
                grants: Mutex::new(Vec::new()),
                device_code_attempts: AtomicU32::new(0),
            }
        }
    }

    impl FirstDeviceAuthorization for 脚本注册 {
        fn attempt<'a>(
            &'a self,
            _prompt: &'a dyn OidcDeviceAuthorizationPromptSink,
        ) -> PortFuture<'a, Result<AuthorizedBridgeDevice, BridgeAuthorizationFailure>> {
            self.device_code_attempts.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { panic!("等电脑授权时不申请设备码") })
        }

        fn attempt_with_grant(
            &self,
            grant: SecretValue,
        ) -> PortFuture<'_, Result<AuthorizedBridgeDevice, BridgeAuthorizationFailure>> {
            self.grants
                .lock()
                .expect("记录锁未中毒")
                .push(grant.expose().to_owned());
            let next = self
                .script
                .lock()
                .expect("脚本锁未中毒")
                .pop()
                .expect("注册次数超出脚本");
            Box::pin(async move { next })
        }
    }

    fn 已授权() -> AuthorizedBridgeDevice {
        AuthorizedBridgeDevice {
            device_id: DeviceId::from_uuid(uuid::Uuid::from_u128(2)),
            access_token_expires_at: UtcMillis::new(301_000).expect("测试时间有效"),
            refresh_token_expires_at: UtcMillis::new(86_401_000).expect("测试时间有效"),
        }
    }

    #[tokio::test]
    async fn 交来电脑授权就拿它注册_不申请设备码() {
        let (reader, mut writer) = io::pipe().expect("可创建管道");
        let mut input = SupervisorInput::watch(reader);
        let authorization = 脚本注册::new([Ok(已授权())]);
        writer
            .write_all(b"hello\ndevice-grant computer-grant\r\n")
            .expect("可写入管道");

        let outcome = authorize_with_supervisor_grant(&authorization, &mut input, |_| {
            panic!("注册成功不报失败")
        })
        .await
        .expect("注册成功");

        assert!(matches!(outcome, GrantOutcome::Authorized(device) if device == 已授权()));
        assert_eq!(
            *authorization.grants.lock().expect("记录锁未中毒"),
            ["computer-grant"]
        );
        assert_eq!(authorization.device_code_attempts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn 注册没成就报失败_接着等下一行() {
        let (reader, mut writer) = io::pipe().expect("可创建管道");
        let mut input = SupervisorInput::watch(reader);
        let authorization = 脚本注册::new([
            Err(super::super::first_authorization_tests::授权被拒().await),
            Ok(已授权()),
        ]);
        writer
            .write_all(b"device-grant first\ndevice-grant second\n")
            .expect("可写入管道");
        let failures = Mutex::new(Vec::new());

        let outcome = authorize_with_supervisor_grant(&authorization, &mut input, |failure| {
            failures
                .lock()
                .expect("失败记录锁未中毒")
                .push(BridgeRuntimeError::authorization(failure).code());
            Ok(())
        })
        .await
        .expect("第二份授权注册成功");

        assert!(matches!(outcome, GrantOutcome::Authorized(_)));
        assert_eq!(
            *failures.lock().expect("失败记录锁未中毒"),
            ["bridge.identity_assertion_invalid"]
        );
        assert_eq!(
            *authorization.grants.lock().expect("记录锁未中毒"),
            ["first", "second"]
        );
    }

    #[tokio::test]
    async fn 桌面端写_device_code_就改走设备码() {
        let (reader, mut writer) = io::pipe().expect("可创建管道");
        let mut input = SupervisorInput::watch(reader);
        let authorization = 脚本注册::new([]);
        writer.write_all(b"device-code\n").expect("可写入管道");

        let outcome = authorize_with_supervisor_grant(&authorization, &mut input, |_| Ok(()))
            .await
            .expect("改走设备码");

        assert!(matches!(outcome, GrantOutcome::DeviceCode));
        // 之后桌面端要退出，照样听得到。
        writer.write_all(b"exit\n").expect("可写入管道");
        let release = tokio::time::timeout(Duration::from_secs(5), input.released())
            .await
            .expect("读到 exit 就应触发退出");
        assert_eq!(release, SupervisorRelease::ExitRequested);
    }

    #[tokio::test]
    async fn 还没授权完桌面端就要换账号_不注册也不申请设备码() {
        let (reader, mut writer) = io::pipe().expect("可创建管道");
        let mut input = SupervisorInput::watch(reader);
        let authorization = 脚本注册::new([]);
        writer.write_all(b"release\r\n").expect("可写入管道");

        let outcome = authorize_with_supervisor_grant(&authorization, &mut input, |_| Ok(()))
            .await
            .expect("要换账号");

        assert!(matches!(outcome, GrantOutcome::ReleaseRequested));
        assert!(
            authorization
                .grants
                .lock()
                .expect("记录锁未中毒")
                .is_empty()
        );
        assert_eq!(authorization.device_code_attempts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn 就绪以后只等_release_这一行_就绪之前写的不算() {
        let (reader, mut writer) = io::pipe().expect("可创建管道");
        let mut input = SupervisorInput::watch(reader);
        writer
            .write_all(b"release\ndevice-code\n")
            .expect("可写入管道");
        // 读线程把两行都交过来以后再丢。
        tokio::time::sleep(Duration::from_millis(200)).await;
        input.discard_pending();

        writer
            .write_all(b"device-grant late\nhello\n")
            .expect("可写入管道");
        assert!(
            tokio::time::timeout(Duration::from_millis(200), input.next_release())
                .await
                .is_err(),
            "就绪之前的 release 和别的行都不算"
        );
        writer.write_all(b"release\n").expect("可写入管道");
        tokio::time::timeout(Duration::from_secs(5), input.next_release())
            .await
            .expect("读到 release");

        // 管道关了不算要移出，交给退出那边。
        drop(writer);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), input.next_release())
                .await
                .is_err()
        );
        assert_eq!(input.released().await, SupervisorRelease::InputClosed);
    }

    #[tokio::test]
    async fn 还没授权完桌面端就退出_bridge_不再等() {
        for (line, expected) in [
            (&b"exit\n"[..], SupervisorRelease::ExitRequested),
            (&b""[..], SupervisorRelease::InputClosed),
        ] {
            let (reader, mut writer) = io::pipe().expect("可创建管道");
            let mut input = SupervisorInput::watch(reader);
            writer.write_all(line).expect("可写入管道");
            drop(writer);

            let outcome =
                authorize_with_supervisor_grant(&脚本注册::new([]), &mut input, |_| Ok(()))
                    .await
                    .expect("桌面端退出");

            assert!(matches!(outcome, GrantOutcome::Released), "{expected:?}");
            assert_eq!(input.released().await, expected);
        }
    }
}
