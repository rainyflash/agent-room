//! 本 crate 唯一的 unsafe 代码：查当前进程的包身份，和带包外策略创建进程。
//!
//! 每个函数只包一组系统调用；分配的属性列表和句柄都在函数返回前释放。
#![allow(unsafe_code)]

use std::{io, marker::PhantomData, mem, ptr};

use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS},
    Storage::Packaging::Appx::GetCurrentPackageFullName,
    System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
        InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
        STARTUPINFOEXW, UpdateProcThreadAttribute,
    },
};

/// `PROCESS_CREATION_DESKTOP_APP_BREAKAWAY_ENABLE_PROCESS_TREE`（winbase.h）：新进程和它的子进程都在包外。
const BREAKAWAY_ENABLE_PROCESS_TREE: u32 = 0x01;

pub(crate) fn inside_app_package() -> bool {
    let mut length = 0_u32;
    // SAFETY: 长度为 0、缓冲区为空时只问需要多长，不写缓冲区；`length` 在调用期间有效。
    let status = unsafe { GetCurrentPackageFullName(&raw mut length, ptr::null_mut()) };
    // 有包时要的长度不够（或长度刚好为 0 时成功）；没有包时是 APPMODEL_ERROR_NO_PACKAGE，别的错误也当没有。
    status == ERROR_INSUFFICIENT_BUFFER || status == ERROR_SUCCESS
}

/// 一份只有包外策略这一项的属性列表；放下时交还系统。列表里存的是 `policy` 的地址，所以不能比它活得久。
struct AttributeList<'policy> {
    // 系统要求列表按指针对齐，所以按 usize 分配。
    storage: Vec<usize>,
    policy: PhantomData<&'policy u32>,
}

impl<'policy> AttributeList<'policy> {
    fn with_breakaway(policy: &'policy u32) -> io::Result<Self> {
        let mut size = 0_usize;
        // SAFETY: 列表为空时只把需要的字节数写进 `size`；这次调用按约定返回失败，所以不看返回值。
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &raw mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut list = Self {
            storage: vec![0; size.div_ceil(size_of::<usize>())],
            policy: PhantomData,
        };
        // SAFETY: `storage` 至少有系统要的 `size` 字节、按指针对齐，在 `Drop` 交还前一直有效。
        if unsafe { InitializeProcThreadAttributeList(list.as_ptr(), 1, 0, &raw mut size) } == 0 {
            // 初始化失败时没有东西可以交还，不能走 Drop。
            list.storage = Vec::new();
            return Err(io::Error::last_os_error());
        }
        // SAFETY: 列表已初始化；`policy` 的借用跟列表的生命周期绑在一起，活得比列表久。
        let updated = unsafe {
            UpdateProcThreadAttribute(
                list.as_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY as usize,
                ptr::from_ref(policy).cast(),
                size_of::<u32>(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if updated == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(list)
    }

    fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
}

impl Drop for AttributeList<'_> {
    fn drop(&mut self) {
        if !self.storage.is_empty() {
            // SAFETY: 列表在 `with_breakaway` 里初始化成功过，这里只交还一次。
            unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
        }
    }
}

/// `application` 是以 NUL 结尾的程序路径，`command_line` 是以 NUL 结尾、可写的整条命令行。
pub(crate) fn create_process_outside_package(
    application: &[u16],
    command_line: &mut [u16],
    flags: u32,
) -> io::Result<u32> {
    debug_assert_eq!(application.last(), Some(&0));
    debug_assert_eq!(command_line.last(), Some(&0));
    let policy = BREAKAWAY_ENABLE_PROCESS_TREE;
    let mut attributes = AttributeList::with_breakaway(&policy)?;
    // SAFETY: 全零是这两个 C 结构合法的初值（指针为空、数字为 0）。
    let mut startup: STARTUPINFOEXW = unsafe { mem::zeroed() };
    startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>())
        .map_err(|_| io::Error::other("STARTUPINFOEXW too large"))?;
    // 标准输入输出都给空句柄：不能把调用方（比如 MCP）的管道交给子进程。
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.lpAttributeList = attributes.as_ptr();
    // SAFETY: 同上，全零是 PROCESS_INFORMATION 合法的初值。
    let mut process: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    // SAFETY: 两个字符串都以 NUL 结尾、在调用期间有效，命令行可写；`startup` 和它指向的属性列表
    // 在调用期间有效；不继承句柄；`process` 由系统填写。
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            flags | EXTENDED_STARTUPINFO_PRESENT,
            ptr::null(),
            ptr::null(),
            (&raw const startup).cast(),
            &raw mut process,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 两个句柄刚由 CreateProcessW 交给我们，各关一次；不等子进程。
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(process.dwProcessId)
}
