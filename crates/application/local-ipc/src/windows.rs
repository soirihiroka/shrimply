use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;
use windows::Win32::Networking::WinSock::{
    ADDRESS_FAMILY, AF_UNIX, FIONBIO, SEND_RECV_FLAGS, SO_RCVTIMEO, SO_SNDTIMEO, SOCK_STREAM,
    SOCKADDR, SOCKADDR_UN, SOCKET, SOCKET_ERROR, SOL_SOCKET, SOMAXCONN, WSADATA, WSAGetLastError,
    WSAStartup, accept, bind, closesocket, ioctlsocket, listen, recv, send, setsockopt, socket,
};

static WINSOCK: OnceLock<Result<(), i32>> = OnceLock::new();

pub struct Listener(SOCKET);
pub struct Stream(SOCKET);

impl Listener {
    pub fn bind(path: &Path) -> io::Result<Self> {
        initialize()?;
        let address = address(path)?;
        let socket =
            unsafe { socket(i32::from(AF_UNIX), SOCK_STREAM, 0) }.map_err(|_| last_error())?;
        if unsafe {
            bind(
                socket,
                (&raw const address).cast::<SOCKADDR>(),
                i32::try_from(std::mem::size_of_val(&address)).expect("socket address fits i32"),
            )
        } == SOCKET_ERROR
            || unsafe { listen(socket, SOMAXCONN as i32) } == SOCKET_ERROR
        {
            let error = last_error();
            close(socket);
            return Err(error);
        }
        Ok(Self(socket))
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        set_nonblocking(self.0, nonblocking)
    }

    pub fn accept(&self) -> io::Result<(Stream, ())> {
        unsafe { accept(self.0, None, None) }
            .map(|socket| (Stream(socket), ()))
            .map_err(|_| last_error())
    }
}

impl Stream {
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        set_nonblocking(self.0, nonblocking)
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        set_timeout(self.0, SO_RCVTIMEO, timeout)
    }

    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        set_timeout(self.0, SO_SNDTIMEO, timeout)
    }
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = unsafe { recv(self.0, buffer, SEND_RECV_FLAGS(0)) };
        if read == SOCKET_ERROR {
            Err(last_error())
        } else {
            Ok(read as usize)
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = unsafe { send(self.0, buffer, SEND_RECV_FLAGS(0)) };
        if written == SOCKET_ERROR {
            Err(last_error())
        } else {
            Ok(written as usize)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        close(self.0);
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        close(self.0);
    }
}

fn initialize() -> io::Result<()> {
    WINSOCK
        .get_or_init(|| {
            let mut data = WSADATA::default();
            let result = unsafe { WSAStartup(0x0202, &mut data) };
            if result == 0 { Ok(()) } else { Err(result) }
        })
        .map_err(|error| io::Error::from_raw_os_error(*error))
}

fn address(path: &Path) -> io::Result<SOCKADDR_UN> {
    let path = path
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "socket path is not Unicode"))?;
    let bytes = path.as_bytes();
    let mut address = SOCKADDR_UN::default();
    if bytes.len() >= address.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket path exceeds the WinSock AF_UNIX limit",
        ));
    }
    address.sun_family = ADDRESS_FAMILY(AF_UNIX);
    for (destination, source) in address.sun_path.iter_mut().zip(bytes) {
        *destination = *source as i8;
    }
    Ok(address)
}

fn set_nonblocking(socket: SOCKET, nonblocking: bool) -> io::Result<()> {
    let mut enabled = u32::from(nonblocking);
    if unsafe { ioctlsocket(socket, FIONBIO, &mut enabled) } == SOCKET_ERROR {
        Err(last_error())
    } else {
        Ok(())
    }
}

fn set_timeout(socket: SOCKET, option: i32, timeout: Option<Duration>) -> io::Result<()> {
    let milliseconds = timeout.map_or(0, |timeout| {
        u32::try_from(timeout.as_millis())
            .unwrap_or(u32::MAX)
            .max(1)
    });
    if unsafe {
        setsockopt(
            socket,
            SOL_SOCKET,
            option,
            Some(&milliseconds.to_ne_bytes()),
        )
    } == SOCKET_ERROR
    {
        Err(last_error())
    } else {
        Ok(())
    }
}

fn last_error() -> io::Error {
    io::Error::from_raw_os_error(unsafe { WSAGetLastError() }.0)
}

fn close(socket: SOCKET) {
    if unsafe { closesocket(socket) } == SOCKET_ERROR {
        std::process::abort();
    }
}
