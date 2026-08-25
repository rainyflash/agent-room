use serde_json::Value;

#[test]
fn 主窗口能力不包含文件_shell_或进程通配权限() {
    let capability: Value = serde_json::from_str(include_str!("../capabilities/main.json"))
        .expect("主窗口能力文件必须是 JSON");
    let permissions = capability["permissions"]
        .as_array()
        .expect("主窗口能力必须声明权限")
        .iter()
        .map(|entry| entry.as_str().expect("权限标识必须是字符串"))
        .collect::<Vec<_>>();

    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    assert!(permissions.iter().all(|permission| {
        !permission.starts_with("fs:")
            && !permission.starts_with("shell:")
            && !permission.starts_with("process:")
            && !permission.contains('*')
    }));
    assert_eq!(
        permissions
            .iter()
            .filter(|permission| permission.starts_with("allow-desktop-"))
            .count(),
        4
    );
}
