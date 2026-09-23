//! 让同一安装里的几个程序不经询问就能读取登录钥匙串里的同一项凭据。
//!
//! 文件钥匙串给每项凭据记一张访问控制表。默认创建的凭据只信任创建它的程序：别的程序读取时，
//! 系统弹窗要登录密码，点“允许”也只放行这一次。这里在创建时就把要读取它的程序写进表里。表里
//! 记的是每个程序的指定要求（designated requirement），不是文件本身，所以同一团队签名的新版本
//! 照样被信任，挪到别的目录也一样；不在表里的程序读取时仍然要经过用户。
//!
//! 已有凭据的访问控制表不能静默修改：`SecKeychainItemSetAccess` 不管调用方是否禁止交互都会弹窗
//! 要钥匙串密码，所以要换访问控制只能删除后重新创建。
#![cfg(target_os = "macos")]

mod ffi;

use std::{ffi::CString, os::unix::ffi::OsStrExt as _, path::Path};

use core_foundation::{
    array::CFArray, base::TCFType as _, data::CFData, dictionary::CFDictionary, string::CFString,
};
use security_framework::{
    base::Result,
    os::macos::keychain::{SecKeychain, SecPreferencesDomain},
};

pub use security_framework::base::Error;

/// 在当前用户的默认钥匙串里新建一项通用密码，调用者和 `readers` 列出的程序都能直接读取。
///
/// 已不存在或无法识别的 `readers` 会被跳过，它们以后读取时照常弹窗。
///
/// # Errors
///
/// 同名凭据已存在、钥匙串不可用或拒绝写入时返回系统错误。
pub fn add_generic_password(
    service: &str,
    account: &str,
    secret: &[u8],
    readers: &[&Path],
) -> Result<()> {
    let mut trusted = vec![ffi::trusted_application(None)?];
    for reader in readers {
        if let Ok(path) = CString::new(reader.as_os_str().as_bytes())
            && let Ok(application) = ffi::trusted_application(Some(&path))
        {
            trusted.push(application);
        }
    }
    let service = CFString::new(service);
    let access = ffi::access(&service, &CFArray::from_CFTypes(&trusted))?;
    // 与 keyring 读写的是同一个钥匙串。
    let keychain = SecKeychain::default_for_domain(SecPreferencesDomain::User)?;
    let key = ffi::item_keys();
    ffi::add_item(&CFDictionary::from_CFType_pairs(&[
        (key.class, key.generic_password.as_CFType()),
        (key.keychain, keychain.as_CFType()),
        (key.service, service.as_CFType()),
        // keyring 创建的凭据也以服务名作标签，钥匙串访问里显示为同一名称。
        (key.label, service.as_CFType()),
        (key.account, CFString::new(account).as_CFType()),
        (key.value, CFData::from_buffer(secret).as_CFType()),
        (key.access, access.as_CFType()),
    ]))
}
