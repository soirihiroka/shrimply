#[cfg(unix)]
pub use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{Listener, Stream};
