//! 子房间跟着父房间公开、改回私密（`specs/redesign/sub-rooms.md` 第 2d-5 步）。
//!
//! 子房间没有自己的公开设置，可它也是一间房：
//!
//! - 父房间改回私密（打开加密）时，还没加密的子房间一起打开加密。有门的本来就加密，剩下的是父房间
//!   公开、不加密时建的没有门的那些。
//! - 父房间公开时，没有门、还没写过 `room.listing.v1` 的子房间写下这一刻：从目录进来的人再进子房间，
//!   别人不给他们重发这以前的房间钥匙，和父房间一样。有门的要批准才进得来，拆门时再写（2d-2）。
//!
//! 两样都先动 Matrix 再记进库里，哪一步没成，再做一次都接得上：写一样的加密状态是幂等的，库里记过
//! “写过这一刻”的不再写。归档了的也跟着改：恢复以后照样是这间房下面的。

use agent_room_domain::time::UtcMillis;

use crate::{
    persistence::RepositoryError,
    ports::{MatrixFailure, PrivateRoomListingMatrix, PrivateRoomListingStore, SubRoomPrivacy},
};

pub(super) enum SubRoomPrivacyFailure {
    Repository(RepositoryError),
    Matrix(MatrixFailure),
}

/// 父房间公开着：没有门、还没写过这一刻的子房间写下 `at`。
pub(super) async fn mark_open_sub_rooms_listed(
    listings: &dyn PrivateRoomListingStore,
    matrix: &dyn PrivateRoomListingMatrix,
    sub_rooms: &[SubRoomPrivacy],
    at: UtcMillis,
) -> Result<(), SubRoomPrivacyFailure> {
    for sub_room in sub_rooms
        .iter()
        .filter(|sub_room| !sub_room.has_door && !sub_room.listing_marked)
    {
        matrix
            .mark_listed(&sub_room.matrix_room, at)
            .await
            .map_err(SubRoomPrivacyFailure::Matrix)?;
        listings
            .mark_sub_room_listed(sub_room.catalog_id, at)
            .await
            .map_err(SubRoomPrivacyFailure::Repository)?;
    }
    Ok(())
}

/// 父房间加密着：还没加密的子房间打开加密，再改目录上的标记。
pub(super) async fn encrypt_sub_rooms(
    listings: &dyn PrivateRoomListingStore,
    matrix: &dyn PrivateRoomListingMatrix,
    sub_rooms: &[SubRoomPrivacy],
) -> Result<(), SubRoomPrivacyFailure> {
    for sub_room in sub_rooms
        .iter()
        .filter(|sub_room| !sub_room.end_to_end_encrypted)
    {
        matrix
            .enable_encryption(&sub_room.matrix_room)
            .await
            .map_err(SubRoomPrivacyFailure::Matrix)?;
        listings
            .mark_room_encrypted(sub_room.catalog_id)
            .await
            .map_err(SubRoomPrivacyFailure::Repository)?;
    }
    Ok(())
}
