//! 本 crate 唯一的 unsafe 代码：查当前进程的包身份，带包外策略创建进程，以及桌面端排队用的
//! 具名互斥体、具名事件和转交参数的窗口消息。
//!
//! 每个函数只包一组系统调用；分配的属性列表和句柄都在函数返回前释放，要留着的句柄交给 `OwnedHandle`。
#![allow(unsafe_code)]

use std::{io, marker::PhantomData, mem, ptr};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HANDLE, WAIT_ABANDONED,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Storage::Packaging::Appx::GetCurrentPackageFullName,
    System::{
        DataExchange::COPYDATASTRUCT,
        Threading::{
            CreateEventW, CreateMutexW, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, InitializeProcThreadAttributeList,
            LPPROC_THREAD_ATTRIBUTE_LIST, OpenEventW, OpenMutexW,
            PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
            STARTUPINFOEXW, SYNCHRONIZATION_SYNCHRONIZE, UpdateProcThreadAttribute,
            WaitForSingleObject,
        },
    },
    UI::WindowsAndMessaging::{FindWindowW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_COPYDATA},
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

/// 一个内核对象句柄，放下时关掉。
pub(crate) struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: 句柄由下面的函数从系统拿到、只属于这个值，这里只关一次。
        unsafe { CloseHandle(self.0) };
    }
}

/// 建一个具名互斥体，已经有了就打开它；建的时候不拿。`name` 以 NUL 结尾。
pub(crate) fn create_mutex(name: &[u16]) -> io::Result<OwnedHandle> {
    debug_assert_eq!(name.last(), Some(&0));
    // SAFETY: 名字以 NUL 结尾、在调用期间有效；不要安全属性，不在建的时候拿。
    let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(OwnedHandle(handle))
}

/// 不等，试着拿互斥体：拿到了，或者上一个拿着它的线程没放就结束了，都算拿到。
/// 拿到的是调用这个函数的线程，在它结束之前一直拿着。
pub(crate) fn try_lock(mutex: &OwnedHandle) -> io::Result<bool> {
    // SAFETY: 句柄在 `mutex` 里有效；超时为 0，不阻塞。
    match unsafe { WaitForSingleObject(mutex.0, 0) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}

/// 有没有进程开着这个名字的互斥体。`name` 以 NUL 结尾。
pub(crate) fn mutex_exists(name: &[u16]) -> bool {
    debug_assert_eq!(name.last(), Some(&0));
    // SAFETY: 名字以 NUL 结尾、在调用期间有效；只要等待的权限。
    let handle = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    drop(OwnedHandle(handle));
    true
}

/// 建一个具名事件（手动复位、不置位），已经有了就打开它。`name` 以 NUL 结尾。
pub(crate) fn create_event(name: &[u16]) -> io::Result<OwnedHandle> {
    debug_assert_eq!(name.last(), Some(&0));
    // SAFETY: 名字以 NUL 结尾、在调用期间有效；不要安全属性。
    let handle = unsafe { CreateEventW(ptr::null(), 1, 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(OwnedHandle(handle))
}

/// 有没有进程开着这个名字的事件。`name` 以 NUL 结尾。
pub(crate) fn event_exists(name: &[u16]) -> bool {
    debug_assert_eq!(name.last(), Some(&0));
    // SAFETY: 名字以 NUL 结尾、在调用期间有效；只要等待的权限。
    let handle = unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    drop(OwnedHandle(handle));
    true
}

/// 找到类名和标题都对得上的顶层窗口，用 `WM_COPYDATA` 把 `data` 交给它，最多等 `timeout_ms`。
/// 找不到窗口、对方卡住或者超时都返回 `false`。`class`、`title` 以 NUL 结尾。
pub(crate) fn send_copy_data(
    class: &[u16],
    title: &[u16],
    kind: usize,
    data: &[u8],
    timeout_ms: u32,
) -> bool {
    debug_assert_eq!(class.last(), Some(&0));
    debug_assert_eq!(title.last(), Some(&0));
    let Ok(length) = u32::try_from(data.len()) else {
        return false;
    };
    // SAFETY: 两个名字以 NUL 结尾、在调用期间有效。
    let window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
    if window.is_null() {
        return false;
    }
    let copy = COPYDATASTRUCT {
        dwData: kind,
        cbData: length,
        lpData: data.as_ptr().cast_mut().cast(),
    };
    let mut result = 0_usize;
    // SAFETY: `copy` 和它指向的 `data` 在调用期间有效；系统把数据复制给对方，对方不能写回。
    // 窗口在找到之后关掉的话，调用只是失败。
    let sent = unsafe {
        SendMessageTimeoutW(
            window,
            WM_COPYDATA,
            0,
            (&raw const copy) as isize,
            SMTO_ABORTIFHUNG,
            timeout_ms,
            &raw mut result,
        )
    };
    sent != 0
}

#[cfg(test)]
mod tests {
    use std::{ptr, sync::Mutex, sync::mpsc};

    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{DataExchange::COPYDATASTRUCT, LibraryLoader::GetModuleHandleW},
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MSG,
            PostMessageW, PostQuitMessage, RegisterClassExW, WM_CLOSE, WM_COPYDATA, WM_DESTROY,
            WNDCLASSEXW, WS_OVERLAPPED,
        },
    };

    use super::send_copy_data;

    /// 窗口收到的 `(dwData, 数据)`。
    static RECEIVED: Mutex<Vec<(usize, Vec<u8>)>> = Mutex::new(Vec::new());

    unsafe extern "system" fn record(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_COPYDATA => {
                // SAFETY: 系统在 WM_COPYDATA 里给的 `lparam` 指向调用期间有效的 COPYDATASTRUCT 和它的数据。
                let data = unsafe {
                    let copy = &*(lparam as *const COPYDATASTRUCT);
                    std::slice::from_raw_parts(copy.lpData.cast::<u8>(), copy.cbData as usize)
                        .to_vec()
                };
                // SAFETY: 同上。
                let kind = unsafe { (*(lparam as *const COPYDATASTRUCT)).dwData };
                RECEIVED.lock().expect("锁没坏").push((kind, data));
                1
            }
            WM_CLOSE => {
                // SAFETY: 窗口是这个线程建的。
                unsafe { DestroyWindow(window) };
                0
            }
            WM_DESTROY => {
                // SAFETY: 在窗口所在的线程上结束它的消息循环。
                unsafe { PostQuitMessage(0) };
                0
            }
            // SAFETY: 别的消息照系统默认处理。
            _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    #[test]
    fn 把数据交给另一个线程上名字对得上的窗口() {
        let class = wide(&format!("agent-room-test-class-{}", std::process::id()));
        let title = wide(&format!("agent-room-test-window-{}", std::process::id()));
        let (ready, window_ready) = mpsc::channel::<isize>();
        let (window_class, window_title) = (class.clone(), title.clone());
        let owner = std::thread::spawn(move || {
            // SAFETY: 类名和标题以 NUL 结尾，在线程结束前一直有效；窗口过程是上面的 `record`。
            unsafe {
                let instance = GetModuleHandleW(ptr::null());
                let mut class = std::mem::zeroed::<WNDCLASSEXW>();
                class.cbSize = u32::try_from(size_of::<WNDCLASSEXW>()).expect("结构不大");
                class.lpfnWndProc = Some(record);
                class.hInstance = instance;
                class.lpszClassName = window_class.as_ptr();
                assert_ne!(RegisterClassExW(&raw const class), 0, "能注册窗口类");
                let window = CreateWindowExW(
                    0,
                    window_class.as_ptr(),
                    window_title.as_ptr(),
                    WS_OVERLAPPED,
                    0,
                    0,
                    0,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    instance,
                    ptr::null(),
                );
                assert!(!window.is_null(), "能建窗口");
                ready.send(window as isize).expect("测试线程在等");
                let mut message = std::mem::zeroed::<MSG>();
                while GetMessageW(&raw mut message, ptr::null_mut(), 0, 0) > 0 {
                    DispatchMessageW(&raw const message);
                }
            }
        });
        let window = window_ready.recv().expect("窗口建好了");

        let payload = br"C:\dir|app.exe|--background"
            .iter()
            .copied()
            .chain([0])
            .collect::<Vec<u8>>();
        assert!(send_copy_data(&class, &title, 1542, &payload, 5_000));
        assert_eq!(
            RECEIVED.lock().expect("锁没坏").as_slice(),
            [(1542, payload)]
        );

        // SAFETY: 窗口还在另一个线程上开着；关窗请求排进它的消息队列。
        unsafe { PostMessageW(window as HWND, WM_CLOSE, 0, 0) };
        owner.join().expect("窗口线程正常结束");
        assert!(
            !send_copy_data(&class, &title, 1542, b"x\0", 1_000),
            "窗口关了以后找不到它"
        );
    }
}
