//! [`Platform`] over the Java host's `com.cluas.moshpit.Platform` object,
//! through JNI. Every call attaches the calling thread to the VM once (the
//! core's threads stay attached) and returns at once: what shows something
//! posts to the main thread on the Java side.
//!
//! A Java exception is caught here: described to logcat, cleared, and
//! turned into an error (`Err(…)`, or the call's empty value).

use nsh::jni::objects::{GlobalRef, JByteArray, JObject, JString, JValue};
use nsh::jni::{JNIEnv, JavaVM};

use crate::platform::{Ask, Notification, Platform};

pub struct JavaPlatform {
    vm: JavaVM,
    object: GlobalRef,
}

impl JavaPlatform {
    pub fn new(env: &mut JNIEnv<'_>, object: &JObject<'_>) -> Option<JavaPlatform> {
        Some(JavaPlatform {
            vm: env.get_java_vm().ok()?,
            object: env.new_global_ref(object).ok()?,
        })
    }

    /// Runs `f` with this thread's JNI env and a local frame.
    fn with<T>(
        &self,
        empty: T,
        f: impl FnOnce(&mut JNIEnv<'_>, &JObject<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut env = match self.vm.attach_current_thread_permanently() {
            Ok(env) => env,
            Err(error) => return Err(format!("cannot attach the thread: {error}")),
        };
        let object = self.object.as_obj();
        let result = env
            .with_local_frame(
                16,
                |env| -> Result<Result<T, String>, nsh::jni::errors::Error> { Ok(f(env, object)) },
            )
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            return Err("the Java side threw".to_owned());
        }
        let _ = empty;
        result
    }

    fn call_void(&self, name: &str, sig: &str, args: &[Arg<'_>]) {
        let result = self.with((), |env, object| {
            let mut owned = Vec::with_capacity(args.len());
            for arg in args {
                owned.push(match arg {
                    Arg::Str(text) => {
                        Held::Object(env.new_string(text).map_err(|e| e.to_string())?.into())
                    }
                    Arg::OptStr(None) => Held::Object(JObject::null()),
                    Arg::OptStr(Some(text)) => {
                        Held::Object(env.new_string(text).map_err(|e| e.to_string())?.into())
                    }
                    Arg::Long(n) => Held::Long(*n),
                    Arg::Bool(flag) => Held::Bool(*flag),
                });
            }
            let values: Vec<JValue<'_, '_>> = owned
                .iter()
                .map(|held| match held {
                    Held::Object(object) => JValue::Object(object),
                    Held::Long(n) => JValue::Long(*n),
                    Held::Bool(flag) => JValue::Bool(u8::from(*flag)),
                })
                .collect();
            env.call_method(object, name, sig, &values)
                .map(drop)
                .map_err(|e| e.to_string())
        });
        if let Err(error) = result {
            log::warn!(target: "moshpit", "Platform.{name}: {error}");
        }
    }
}

/// An argument of a void call.
enum Arg<'a> {
    Str(&'a str),
    OptStr(Option<&'a str>),
    Long(i64),
    Bool(bool),
}

/// An argument made into a Java value.
enum Held<'a> {
    Object(JObject<'a>),
    Long(i64),
    Bool(bool),
}

/// A Java `String` result (`None` for null).
fn read_string(env: &mut JNIEnv<'_>, object: JObject<'_>) -> Option<String> {
    if object.is_null() {
        return None;
    }
    let text = JString::from(object);
    env.get_string(&text).ok().map(String::from)
}

/// A Java `byte[]` result (`None` for null).
fn read_bytes(env: &mut JNIEnv<'_>, object: JObject<'_>) -> Option<Vec<u8>> {
    if object.is_null() {
        return None;
    }
    let array = JByteArray::from(object);
    env.convert_byte_array(&array).ok()
}

/// Calls a method returning an object; the exception, if one is thrown,
/// as `Err` with its `toString()`.
fn call_object<'a>(
    env: &mut JNIEnv<'a>,
    object: &JObject<'_>,
    name: &str,
    sig: &str,
    args: &[JValue<'_, '_>],
) -> Result<JObject<'a>, String> {
    match env.call_method(object, name, sig, args) {
        Ok(value) => value.l().map_err(|e| e.to_string()),
        Err(error) => {
            if env.exception_check().unwrap_or(false) {
                let thrown = env.exception_occurred().ok();
                let _ = env.exception_clear();
                if let Some(thrown) = thrown {
                    let text = env
                        .call_method(&thrown, "toString", "()Ljava/lang/String;", &[])
                        .and_then(|v| v.l())
                        .ok()
                        .and_then(|o| read_string(env, o));
                    return Err(text.unwrap_or_else(|| error.to_string()));
                }
            }
            Err(error.to_string())
        }
    }
}

impl Platform for JavaPlatform {
    fn pref_get(&self, key: &str) -> Option<String> {
        self.with(None, |env, object| {
            let key = env.new_string(key).map_err(|e| e.to_string())?;
            let value = call_object(
                env,
                object,
                "prefGet",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&key)],
            )?;
            Ok(read_string(env, value))
        })
        .unwrap_or_else(|error| {
            log::warn!(target: "moshpit", "prefGet {key}: {error}");
            None
        })
    }

    fn pref_set(&self, key: &str, value: Option<&str>) {
        self.call_void(
            "prefSet",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[Arg::Str(key), Arg::OptStr(value)],
        );
    }

    fn user_unlocked(&self) -> bool {
        self.with(true, |env, object| {
            env.call_method(object, "userUnlocked", "()Z", &[])
                .and_then(|v| v.z())
                .map_err(|e| e.to_string())
        })
        .unwrap_or(true)
    }

    fn secret_save(&self, account: &str, secret: &[u8], biometry: bool) -> Result<(), String> {
        self.with((), |env, object| {
            let account = env.new_string(account).map_err(|e| e.to_string())?;
            let secret = env
                .byte_array_from_slice(secret)
                .map_err(|e| e.to_string())?;
            let failure = call_object(
                env,
                object,
                "secretSave",
                "(Ljava/lang/String;[BZ)Ljava/lang/String;",
                &[
                    JValue::Object(&account),
                    JValue::Object(&secret),
                    JValue::Bool(u8::from(biometry)),
                ],
            )?;
            match read_string(env, failure) {
                None => Ok(()),
                Some(why) => Err(why),
            }
        })
    }

    fn secret_load(&self, account: &str) -> Result<Option<Vec<u8>>, String> {
        self.with(None, |env, object| {
            let account = env.new_string(account).map_err(|e| e.to_string())?;
            let value = call_object(
                env,
                object,
                "secretLoad",
                "(Ljava/lang/String;)[B",
                &[JValue::Object(&account)],
            )?;
            Ok(read_bytes(env, value))
        })
    }

    fn secret_delete(&self, account: &str) -> Result<(), String> {
        self.with((), |env, object| {
            let account = env.new_string(account).map_err(|e| e.to_string())?;
            let failure = call_object(
                env,
                object,
                "secretDelete",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&account)],
            )?;
            match read_string(env, failure) {
                None => Ok(()),
                Some(why) => Err(why),
            }
        })
    }

    fn secret_exists(&self, account: &str) -> bool {
        self.with(false, |env, object| {
            let account = env.new_string(account).map_err(|e| e.to_string())?;
            env.call_method(
                object,
                "secretExists",
                "(Ljava/lang/String;)Z",
                &[JValue::Object(&account)],
            )
            .and_then(|v| v.z())
            .map_err(|e| e.to_string())
        })
        .unwrap_or(false)
    }

    fn hardware_generate(&self, alias: &str) -> Result<Vec<u8>, String> {
        self.with(Vec::new(), |env, object| {
            let alias = env.new_string(alias).map_err(|e| e.to_string())?;
            let value = call_object(
                env,
                object,
                "hardwareGenerate",
                "(Ljava/lang/String;)[B",
                &[JValue::Object(&alias)],
            )?;
            read_bytes(env, value).ok_or_else(|| "the Keystore made no key".to_owned())
        })
    }

    fn hardware_sign(&self, alias: &str, data: &[u8]) -> Result<Vec<u8>, String> {
        self.with(Vec::new(), |env, object| {
            let alias = env.new_string(alias).map_err(|e| e.to_string())?;
            let data = env.byte_array_from_slice(data).map_err(|e| e.to_string())?;
            let value = call_object(
                env,
                object,
                "hardwareSign",
                "(Ljava/lang/String;[B)[B",
                &[JValue::Object(&alias), JValue::Object(&data)],
            )?;
            read_bytes(env, value).ok_or_else(|| "the Keystore signed nothing".to_owned())
        })
    }

    fn hardware_delete(&self, alias: &str) {
        self.call_void(
            "hardwareDelete",
            "(Ljava/lang/String;)V",
            &[Arg::Str(alias)],
        );
    }

    fn push(&self, surface: u64, animated: bool) {
        self.call_void(
            "push",
            "(JZ)V",
            &[Arg::Long(surface as i64), Arg::Bool(animated)],
        );
    }

    fn pop(&self, surface: u64, animated: bool) {
        self.call_void(
            "pop",
            "(JZ)V",
            &[Arg::Long(surface as i64), Arg::Bool(animated)],
        );
    }

    fn finish(&self) {
        self.call_void("finish", "()V", &[]);
    }

    fn copy_text(&self, text: &str) {
        self.call_void("copyText", "(Ljava/lang/String;)V", &[Arg::Str(text)]);
    }

    fn share_text(&self, text: &str) {
        self.call_void("shareText", "(Ljava/lang/String;)V", &[Arg::Str(text)]);
    }

    fn open_url(&self, url: &str) {
        self.call_void("openUrl", "(Ljava/lang/String;)V", &[Arg::Str(url)]);
    }

    fn toast(&self, text: &str) {
        self.call_void("toast", "(Ljava/lang/String;)V", &[Arg::Str(text)]);
    }

    fn notify(&self, n: &Notification) {
        self.call_void(
            "notify",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
            &[
                Arg::Str(&n.channel),
                Arg::Str(&n.tag),
                Arg::Str(&n.title),
                Arg::Str(&n.body),
                Arg::Str(&n.link),
            ],
        );
    }

    fn ask(&self, ask: &Ask) {
        match ask {
            Ask::Confirm {
                token,
                title,
                message,
                ok,
                destructive,
            } => self.call_void(
                "confirm",
                "(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;Z)V",
                &[
                    Arg::Long(*token as i64),
                    Arg::Str(title),
                    Arg::Str(message),
                    Arg::Str(ok),
                    Arg::Bool(*destructive),
                ],
            ),
            Ask::Prompt {
                token,
                title,
                text,
                ok,
                multiline,
            } => self.call_void(
                "prompt",
                "(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;Z)V",
                &[
                    Arg::Long(*token as i64),
                    Arg::Str(title),
                    Arg::Str(text),
                    Arg::Str(ok),
                    Arg::Bool(*multiline),
                ],
            ),
        }
    }

    fn release_stream(&self, stream: u64) {
        self.call_void("releaseStream", "(J)V", &[Arg::Long(stream as i64)]);
    }

    fn show_stream(&self, surface: u64, stream: u64) {
        self.call_void(
            "showStream",
            "(JJ)V",
            &[Arg::Long(surface as i64), Arg::Long(stream as i64)],
        );
    }

    fn terminal_look(&self, colors: &[String], font_size: f64) {
        let result = self.with((), |env, object| {
            let class = env
                .find_class("java/lang/String")
                .map_err(|e| e.to_string())?;
            let array = env
                .new_object_array(colors.len() as i32, &class, JObject::null())
                .map_err(|e| e.to_string())?;
            for (i, color) in colors.iter().enumerate() {
                let text = env.new_string(color).map_err(|e| e.to_string())?;
                env.set_object_array_element(&array, i as i32, &text)
                    .map_err(|e| e.to_string())?;
            }
            env.call_method(
                object,
                "terminalLook",
                "([Ljava/lang/String;D)V",
                &[JValue::Object(&array), JValue::Double(font_size)],
            )
            .map(drop)
            .map_err(|e| e.to_string())
        });
        if let Err(error) = result {
            log::warn!(target: "moshpit", "Platform.terminalLook: {error}");
        }
    }

    fn keyboard(&self, show: bool) {
        self.call_void("keyboard", "(Z)V", &[Arg::Bool(show)]);
    }

    fn set_ctrl(&self, armed: bool) {
        self.call_void("setCtrl", "(Z)V", &[Arg::Bool(armed)]);
    }

    fn paste(&self) {
        self.call_void("paste", "()V", &[]);
    }

    fn pick_file(&self, token: u64) {
        self.call_void("pickFile", "(J)V", &[Arg::Long(token as i64)]);
    }

    fn fill_key_field(&self, kind: &str, text: &str) {
        self.call_void(
            "fillKeyField",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[Arg::Str(kind), Arg::Str(text)],
        );
    }
}
