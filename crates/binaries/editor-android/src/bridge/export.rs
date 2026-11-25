use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{JObject, JString},
    refs::Global,
};
use std::{fs::File, os::fd::BorrowedFd, sync::OnceLock};

static STORAGE: OnceLock<(JavaVM, Global<JObject<'static>>)> = OnceLock::new();

pub(super) fn initialize(vm: JavaVM, timeline: Global<JObject<'static>>) {
    STORAGE.get_or_init(|| (vm, timeline));
}

pub(crate) fn write_document(
    uri: &str,
    mime: &str,
    filename: Option<&str>,
    write: impl FnOnce(&mut dyn FnMut() -> Result<File, String>) -> Result<(), String>,
) -> Result<String, String> {
    let (vm, timeline) = STORAGE.get().expect("initialized Android export storage");
    let application = super::APPLICATION
        .get()
        .expect("initialized Android context");
    vm
            .attach_current_thread(|env| -> jni::errors::Result<Result<String, String>> {
                let result = (|| {
                    let uri = env.new_string(uri)?;
                    let mime = env.new_string(mime)?;
                    let filename = env.new_string(filename.unwrap_or_default())?;
                    env.call_method(
                        timeline,
                        jni_str!("exportDocument"),
                        jni_sig!("(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ldev/shrimply/app/ExportDocument;"),
                        &[application.as_obj().into(), (&uri).into(), (&mime).into(), (&filename).into()],
                    )?.l()
                })();
                let document = match result {
                    Ok(document) => document,
                    Err(error) => return Err(env.exception_catch().err().unwrap_or(error)),
                };
                let name = (|| {
                    let name = env.call_method(&document, jni_str!("name"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
                    env.cast_local::<JString>(name)?.try_to_string(env)
                })();
                let output = match name {
                    Ok(name) => write(&mut || {
                        let fd = env.call_method(&document, jni_str!("open"), jni_sig!("()I"), &[])
                            .and_then(|fd| fd.i())
                            .map_err(|error| provider_error(env.exception_catch().err().unwrap_or(error)))?;
                        // Keep the provider descriptor for its close/error signal.
                        unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned()
                            .map(File::from)
                            .map_err(|error| error.to_string())
                    }).map(|()| name),
                    Err(error) => Err(provider_error(env.exception_catch().err().unwrap_or(error))),
                };
                let finish = env.call_method(
                    &document,
                    jni_str!("finish"),
                    jni_sig!("(Z)V"),
                    &[output.is_ok().into()],
                );
                Ok(match finish {
                    Ok(_) => output,
                    Err(error) => {
                        let finish = provider_error(env.exception_catch().err().unwrap_or(error));
                        Err(match output {
                            Ok(_) => finish,
                            Err(error) => format!("{error}\n{finish}"),
                        })
                    }
                })
            })
            .map_err(provider_error)?
}

fn provider_error(error: jni::errors::Error) -> String {
    match error {
        jni::errors::Error::CaughtJavaException { msg, .. } => msg,
        error => error.to_string(),
    }
}
