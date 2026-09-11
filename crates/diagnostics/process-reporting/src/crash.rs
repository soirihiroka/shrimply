use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

static LAST_CONTEXT: Mutex<Option<String>> = Mutex::new(None);
const SIGNAL_CONTEXT_CAP: usize = 384;
const SIGNAL_CONTEXT_HISTORY: usize = 8;
static SIGNAL_CONTEXT_WRITTEN: AtomicUsize = AtomicUsize::new(0);
static SIGNAL_CONTEXT_LENS: [AtomicUsize; SIGNAL_CONTEXT_HISTORY] =
    [const { AtomicUsize::new(0) }; SIGNAL_CONTEXT_HISTORY];
static SIGNAL_CONTEXTS: [[AtomicU8; SIGNAL_CONTEXT_CAP]; SIGNAL_CONTEXT_HISTORY] =
    [const { [const { AtomicU8::new(0) }; SIGNAL_CONTEXT_CAP] }; SIGNAL_CONTEXT_HISTORY];

pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        let payload = panic_payload(info);
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| String::from("unknown"));
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");
        let context = last_context();
        let backtrace = Backtrace::force_capture();

        eprintln!(
            "shrimply crash: panic at {location} on thread {thread_name} {:?}: {payload}\nlast context: {context}\nbacktrace:\n{backtrace}",
            thread.id()
        );
        tracing::error!(
            "panic at {location} on thread {thread_name} {:?}: {payload}; last context: {context}\nbacktrace:\n{backtrace}",
            thread.id()
        );
    }));

    #[cfg(unix)]
    install_native_handlers();
}

#[cfg(unix)]
fn install_native_handlers() {
    for signal in [
        libc::SIGABRT,
        libc::SIGBUS,
        libc::SIGFPE,
        libc::SIGILL,
        libc::SIGSEGV,
        libc::SIGTRAP,
    ] {
        install_signal_handler(signal);
    }
}

pub fn set_context(context: impl Into<String>) {
    let context = context.into();
    store_signal_context(&context);
    match LAST_CONTEXT.lock() {
        Ok(mut last_context) => *last_context = Some(context),
        Err(error) => *error.into_inner() = Some(context),
    }
}

fn store_signal_context(context: &str) {
    let slot = SIGNAL_CONTEXT_WRITTEN.load(Ordering::SeqCst) % SIGNAL_CONTEXT_HISTORY;
    SIGNAL_CONTEXT_LENS[slot].store(0, Ordering::SeqCst);
    let bytes = context.as_bytes();
    let len = bytes.len().min(SIGNAL_CONTEXT_CAP);
    for (index, byte) in bytes.iter().take(len).enumerate() {
        SIGNAL_CONTEXTS[slot][index].store(*byte, Ordering::SeqCst);
    }
    SIGNAL_CONTEXT_LENS[slot].store(len, Ordering::SeqCst);
    SIGNAL_CONTEXT_WRITTEN.fetch_add(1, Ordering::SeqCst);
}

fn panic_payload(info: &PanicHookInfo<'_>) -> String {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = info.payload().downcast_ref::<String>() {
        return message.clone();
    }
    String::from("non-string panic payload")
}

fn last_context() -> String {
    match LAST_CONTEXT.try_lock() {
        Ok(context) => context
            .as_deref()
            .unwrap_or("no app context recorded")
            .to_string(),
        Err(std::sync::TryLockError::Poisoned(error)) => error
            .into_inner()
            .as_deref()
            .unwrap_or("crash context was poisoned")
            .to_string(),
        Err(std::sync::TryLockError::WouldBlock) => String::from("crash context lock busy"),
    }
}

#[cfg(unix)]
fn install_signal_handler(signal: i32) {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_flags = libc::SA_RESETHAND;
        action.sa_sigaction = fatal_signal_handler as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
            tracing::error!("Could not register crash handler for signal {signal}");
        }
    }
}

#[cfg(unix)]
extern "C" fn fatal_signal_handler(signal: i32) {
    unsafe {
        write_stderr(signal_message(signal));
        write_signal_context();

        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut action.sa_mask);
        let _ = libc::sigaction(signal, &action, std::ptr::null_mut());

        let mut mask: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut mask);
        libc::sigaddset(&mut mask, signal);
        let _ = libc::sigprocmask(libc::SIG_UNBLOCK, &mask, std::ptr::null_mut());
        libc::raise(signal);
        libc::_exit(128 + signal);
    }
}

#[cfg(unix)]
fn signal_message(signal: i32) -> &'static [u8] {
    match signal {
        libc::SIGABRT => b"\nshrimply crash: received SIGABRT.\n",
        libc::SIGBUS => b"\nshrimply crash: received SIGBUS.\n",
        libc::SIGFPE => b"\nshrimply crash: received SIGFPE.\n",
        libc::SIGILL => b"\nshrimply crash: received SIGILL.\n",
        libc::SIGSEGV => b"\nshrimply crash: received SIGSEGV.\n",
        libc::SIGTRAP => b"\nshrimply crash: received SIGTRAP.\n",
        _ => b"\nshrimply crash: received fatal signal.\n",
    }
}

#[cfg(unix)]
unsafe fn write_signal_context() {
    unsafe {
        let written = SIGNAL_CONTEXT_WRITTEN.load(Ordering::SeqCst);
        let count = written.min(SIGNAL_CONTEXT_HISTORY);
        if count == 0 {
            write_stderr(b"recent contexts: none recorded\n");
            return;
        }

        write_stderr(b"recent contexts:\n");
        for index in written.saturating_sub(count)..written {
            let slot = index % SIGNAL_CONTEXT_HISTORY;
            let len = SIGNAL_CONTEXT_LENS[slot]
                .load(Ordering::SeqCst)
                .min(SIGNAL_CONTEXT_CAP);
            write_stderr(b"  - ");
            if len == 0 {
                write_stderr(b"context write in progress");
            } else {
                for byte in SIGNAL_CONTEXTS[slot].iter().take(len) {
                    let byte = byte.load(Ordering::SeqCst);
                    let _ = libc::write(libc::STDERR_FILENO, (&byte as *const u8).cast(), 1);
                }
            }
            write_stderr(b"\n");
        }
    }
}

#[cfg(unix)]
unsafe fn write_stderr(message: &[u8]) {
    unsafe {
        let _ = libc::write(libc::STDERR_FILENO, message.as_ptr().cast(), message.len());
    }
}
