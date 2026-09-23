//! 整个工作区唯一的 unsafe 代码：`security-framework` 没有封装的几个 Security 框架调用。
//!
//! 每个函数只做一次调用，并按 Core Foundation 的所有权规则把结果包成会自动释放的对象。
#![allow(unsafe_code)]

use std::{
    ffi::{CStr, c_char},
    ptr,
};

use core_foundation::{
    array::{CFArray, CFArrayRef},
    base::{CFType, CFTypeRef, OSStatus, TCFType as _},
    dictionary::CFDictionary,
    string::{CFString, CFStringRef},
};
use security_framework::{
    base::{Error, Result},
    os::macos::access::SecAccess,
};
use security_framework_sys::{
    base::{SecAccessRef, errSecSuccess},
    item::{
        kSecAttrAccount, kSecAttrLabel, kSecAttrService, kSecClass, kSecClassGenericPassword,
        kSecUseKeychain, kSecValueData,
    },
    keychain_item::SecItemAdd,
};

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecAttrAccess: CFStringRef;
    fn SecTrustedApplicationCreateFromPath(path: *const c_char, app: *mut CFTypeRef) -> OSStatus;
    fn SecAccessCreate(
        descriptor: CFStringRef,
        trusted_list: CFArrayRef,
        access: *mut SecAccessRef,
    ) -> OSStatus;
}

/// 访问控制表里的一个受信程序；`None` 表示调用者自己。
pub(crate) fn trusted_application(path: Option<&CStr>) -> Result<CFType> {
    let mut application: CFTypeRef = ptr::null();
    // SAFETY: `path` 为空或是在调用期间有效、以 NUL 结尾的字符串；函数只在成功时写入 `application`。
    let status = unsafe {
        SecTrustedApplicationCreateFromPath(
            path.map_or(ptr::null(), CStr::as_ptr),
            &raw mut application,
        )
    };
    check(status)?;
    // SAFETY: 成功时 `application` 是按 Create 规则返回、归调用方所有的非空引用。
    Ok(unsafe { CFType::wrap_under_create_rule(application) })
}

/// 只信任 `trusted` 列出程序的访问控制；`descriptor` 是系统弹窗里显示的凭据名称。
pub(crate) fn access(descriptor: &CFString, trusted: &CFArray<CFType>) -> Result<SecAccess> {
    let mut access: SecAccessRef = ptr::null_mut();
    // SAFETY: 两个参数在调用期间有效；函数只在成功时写入 `access`。
    let status = unsafe {
        SecAccessCreate(
            descriptor.as_concrete_TypeRef(),
            trusted.as_concrete_TypeRef(),
            &raw mut access,
        )
    };
    check(status)?;
    // SAFETY: 成功时 `access` 是按 Create 规则返回、归调用方所有的非空引用。
    Ok(unsafe { SecAccess::wrap_under_create_rule(access) })
}

pub(crate) fn add_item(attributes: &CFDictionary<CFString, CFType>) -> Result<()> {
    // SAFETY: 属性字典在调用期间有效；不索取返回值，结果指针可以为空。
    check(unsafe { SecItemAdd(attributes.as_concrete_TypeRef(), ptr::null_mut()) })
}

pub(crate) struct ItemKeys {
    pub(crate) class: CFString,
    pub(crate) generic_password: CFString,
    pub(crate) keychain: CFString,
    pub(crate) service: CFString,
    pub(crate) label: CFString,
    pub(crate) account: CFString,
    pub(crate) value: CFString,
    pub(crate) access: CFString,
}

pub(crate) fn item_keys() -> ItemKeys {
    // SAFETY: 这些都是 Security 框架导出、在进程生命周期内不变的非空 CFString 常量。
    unsafe {
        ItemKeys {
            class: CFString::wrap_under_get_rule(kSecClass),
            generic_password: CFString::wrap_under_get_rule(kSecClassGenericPassword),
            keychain: CFString::wrap_under_get_rule(kSecUseKeychain),
            service: CFString::wrap_under_get_rule(kSecAttrService),
            label: CFString::wrap_under_get_rule(kSecAttrLabel),
            account: CFString::wrap_under_get_rule(kSecAttrAccount),
            value: CFString::wrap_under_get_rule(kSecValueData),
            access: CFString::wrap_under_get_rule(kSecAttrAccess),
        }
    }
}

fn check(status: OSStatus) -> Result<()> {
    if status == errSecSuccess {
        Ok(())
    } else {
        Err(Error::from_code(status))
    }
}
