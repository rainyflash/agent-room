//! 恢复归档了的子房间在真实 Synapse 上（`specs/redesign/sub-rooms.md` 的“归档和恢复”，2e-3）。
//!
//! 归档锁住以后谁也说不了（上一层 `sub_rooms.rs` 里查过）。恢复用 `reopen` 一次写好私人房间的门槛和
//! 每个人的级别：照给的说得了、说不了，没给的人留着原来的级别，一个也没给也照样解开门槛。控制面照
//! 此刻的成员记录和禁言给（`sub_room_sync`），这里只查聊天服务器上是不是那个意思。

use super::*;

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 私人房间配置"]
async fn 真实_synapse_恢复归档的子房间_原来的人接着说_被禁言的照旧说不了_重做不出错() {
    let family = family().await;
    let [archived, open] = &family.sub_rooms;
    let owner_id = family.owner.session().metadata().user_id().clone();

    family
        .provisioner
        .archive(archived)
        .await
        .expect("能锁住归档的房间");
    refused(&family.owner, archived).await;
    refused(&family.member, archived).await;

    // 恢复：房主接着说；成员这会儿被禁言着，照旧说不了。别的房间不受影响，重做一遍不出错。
    let assignments = [
        PrivateMatrixSpeakingAssignment::new(owner_id, true),
        PrivateMatrixSpeakingAssignment::new(family.member_id.clone(), false),
    ];
    for _ in 0..2 {
        PrivateRoomMatrixGateway::reopen(&*family.provisioner, archived, &assignments)
            .await
            .expect("能解开，再解一次不出错");
    }
    speak(&family.owner, archived).await;
    refused(&family.member, archived).await;
    speak(&family.member, open).await;

    // 禁言放开了，对账给他发言级别：他接着说。
    family
        .provisioner
        .set_speaking(archived, &family.member_id, true)
        .await
        .expect("能给回发言级别");
    speak(&family.member, archived).await;

    // 再归档、再恢复，这次一个人也没给（控制面那边里面已经没有记着的人）：门槛照样解开，没给的人
    // 留着原来的级别。
    family
        .provisioner
        .archive(archived)
        .await
        .expect("能再锁一次");
    refused(&family.owner, archived).await;
    PrivateRoomMatrixGateway::reopen(&*family.provisioner, archived, &[])
        .await
        .expect("没给人也能解开");
    speak(&family.owner, archived).await;
    speak(&family.member, archived).await;
}
