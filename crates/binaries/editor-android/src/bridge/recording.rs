//! JNI bindings for the MediaProjection worker and its encoder input surface.
use jni::{
    Env, EnvUnowned,
    errors::{Error, ThrowRuntimeExAndDefault},
    jni_str,
    objects::{JObject, JString},
    strings::JNIString,
    sys::{jboolean, jlong},
};
use shrimply_export_core::android::recording::Recording;
use std::cell::RefCell;

thread_local! {
    static CAPTURE: RefCell<Option<Recording>> = RefCell::default();
}

fn checked<T>(env: &mut Env<'_>, result: Result<T, String>) -> jni::errors::Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            env.throw_new(
                jni_str!("java/lang/IllegalStateException"),
                JNIString::new(error),
            )?;
            Err(Error::JavaException)
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeScreenRecorder_open<'local>(
    mut raw: EnvUnowned<'local>,
    _this: JObject<'local>,
    config: JString<'local>,
) -> JObject<'local> {
    raw.with_env(|env| {
        let config = config.try_to_string(env)?;
        let result = (|| {
            if CAPTURE.with_borrow(Option::is_some) {
                return Err("Screen recording already open".into());
            }
            let settings = serde_json::from_str(&config)
                .map_err(|error| format!("Invalid screen recording: {error}"))?;
            let (recording, window) = Recording::open(settings)?;
            let surface = unsafe { window.to_surface(env.get_raw().cast()) };
            if surface.is_null() {
                return Err("Could not create the screen capture Surface".into());
            }
            CAPTURE.set(Some(recording));
            Ok(unsafe { JObject::from_raw(env, surface.cast()) })
        })();
        checked(env, result)
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeScreenRecorder_poll<'local>(
    mut raw: EnvUnowned<'local>,
    _this: JObject<'local>,
) -> jboolean {
    raw.with_env(|env| {
        checked(
            env,
            CAPTURE.with_borrow_mut(|recording| {
                recording
                    .as_mut()
                    .ok_or("Screen recording is closed")?
                    .poll()?
                    .map(|packet| packet as jboolean)
                    .ok_or_else(|| "Screen encoder ended before capture stopped".into())
            }),
        )
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeScreenRecorder_finish<'local>(
    mut raw: EnvUnowned<'local>,
    _this: JObject<'local>,
    elapsed_us: jlong,
) -> jlong {
    raw.with_env(|env| {
        checked(
            env,
            CAPTURE
                .take()
                .ok_or_else(|| "Screen recording is closed".into())
                .and_then(|recording| recording.finish(elapsed_us)),
        )
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeScreenRecorder_close(
    _env: EnvUnowned<'_>,
    _this: JObject<'_>,
) {
    CAPTURE.take();
}
