use std::time::Duration;

use agent_room_application::ports::AutomationContentScanner;
use agent_room_content_adapter::{ClamAvAutomationScanner, ClamAvScannerConfig};
use agent_room_domain::{content::ContentScanState, policy::AutomationMessageText};

#[tokio::test]
#[ignore = "需要隔离环境中的真实 ClamAV 服务"]
async fn 自动回复通过真实_clamav_扫描() {
    let address = std::env::var("AGENT_ROOM_TEST_SCANNER_ADDRESS").expect("必须提供测试扫描器地址");
    let config = ClamAvScannerConfig::new(address, Duration::from_secs(5), Duration::from_secs(15))
        .expect("测试扫描器须为私网端点");
    let scanner = ClamAvAutomationScanner::new(config);
    let text = AutomationMessageText::new(
        "建议按任务领域和当前接待状态寻找 Agent。\n这是经真实扫描服务检查的测试回复。".to_owned(),
    )
    .expect("正文有效");
    assert_eq!(
        scanner.scan(&text).await.expect("真实扫描完成"),
        ContentScanState::Clean
    );
}
