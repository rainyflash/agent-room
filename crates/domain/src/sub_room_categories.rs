//! 子房间的分类（`specs/redesign/sub-rooms.md` 的“分类”，第 2e-1 步）：给子房间分的组，整间房共用，
//! 只有名字和先后。父房间的房主和管理员建、改名、排先后、删；一间房最多 20 个，名字不重名（不分
//! 大小写）。分类只管怎么摆，不管权限。

use crate::{
    DomainError, DomainResult, ids::PrincipalId, private_rooms::PrivateRoom,
    sub_rooms::runs_parent_room,
};

/// 一间房最多几个分类。
pub const MAXIMUM_SUB_ROOM_CATEGORIES: u8 = 20;

/// 分类的名字最多几个字。库里的约束也卡着同一个数。
pub const MAXIMUM_SUB_ROOM_CATEGORY_NAME_CHARS: usize = 64;

/// 分类的名字：不是空的，两头不带空白，最多 64 个字，不带换行这类控制字符。
///
/// # Errors
///
/// 不合规矩时返回错误。
pub fn validate_sub_room_category_name(name: &str) -> DomainResult<()> {
    let reason = if name.is_empty() || name != name.trim() {
        Some("名字不能是空的，两头不带空白")
    } else if name.chars().count() > MAXIMUM_SUB_ROOM_CATEGORY_NAME_CHARS
        // 转成小写以后个别字会变长，比重名用的键也得放得下。
        || sub_room_category_name_key(name).chars().count() > MAXIMUM_SUB_ROOM_CATEGORY_NAME_CHARS
    {
        Some("名字最多 64 个字")
    } else if name.chars().any(char::is_control) {
        Some("名字里不能有换行这类控制字符")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(DomainError::Validation {
            field: "sub_room_category_name",
            reason,
        }),
        None => Ok(()),
    }
}

/// 比重名用的键：去掉两头的空白、转成小写，和子房间的名字一样。
#[must_use]
pub fn sub_room_category_name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// 谁能建、改、删分类：父房间的房主和管理员。
#[must_use]
pub fn arranges_sub_room_categories(parent: &PrivateRoom, principal: PrincipalId) -> bool {
    runs_parent_room(parent, principal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 名字不能是空的_两头不带空白_最多六十四个字_不带控制字符() {
        for valid in ["发布", "Design review", &"字".repeat(64)] {
            assert!(validate_sub_room_category_name(valid).is_ok(), "{valid}");
        }
        for invalid in ["", " 发布", "发布 ", "发\n布", "a\tb", &"字".repeat(65)] {
            assert!(
                validate_sub_room_category_name(invalid).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn 比重名不分大小写() {
        assert_eq!(
            sub_room_category_name_key("Design"),
            sub_room_category_name_key("design")
        );
        assert_ne!(
            sub_room_category_name_key("设计"),
            sub_room_category_name_key("发布")
        );
    }
}
