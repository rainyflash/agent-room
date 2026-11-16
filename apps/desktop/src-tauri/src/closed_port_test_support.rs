//! 测试里要一个“没人听”的控制面地址时用。
//!
//! 别用“绑定一个端口、读出地址再放掉”：放掉的端口马上会被同一个测试程序里并行的别的用例
//! 拿去起服务，请求就打到那边去了。2026-11-15 的 Windows 原生检查就这样偶发红过一次：
//! 电脑授权“连不上”的用例连上了代发用例只接一个连接的服务，两边一起失败。
//! 这里占着端口但不 `listen`：连过来一律被拒，拿着它的期间别的用例也绑不到这个端口。

use std::net::{Ipv4Addr, SocketAddr};

use tokio::net::TcpSocket;

/// 拿着期间端口一直占着、没人听；丢掉才放开。
#[must_use = "丢掉就放开端口，别的用例可能马上拿去起服务"]
pub(crate) struct ClosedPort {
    _socket: TcpSocket,
    address: SocketAddr,
}

impl ClosedPort {
    pub(crate) fn reserve() -> Self {
        let socket = TcpSocket::new_v4().expect("可建出 TCP 套接字");
        socket
            .bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .expect("可绑定回环端口");
        let address = socket.local_addr().expect("可读本机地址");
        Self {
            _socket: socket,
            address,
        }
    }

    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }
}

#[tokio::test]
async fn 拿着期间别人绑不到_连过来被拒() {
    let closed = ClosedPort::reserve();
    assert!(
        tokio::net::TcpListener::bind(closed.address())
            .await
            .is_err(),
        "占着的端口别人不该绑得到"
    );
    assert!(
        tokio::net::TcpStream::connect(closed.address())
            .await
            .is_err(),
        "没人听，连过来应被拒"
    );
}
