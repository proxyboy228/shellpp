//! The core's platform services backed by the app's Kotlin object
//! `dev.shellpp.android.Platform` (architecture 2 and 4): the device key in
//! the Android Keystore behind the biometric prompt (D218), the clipboard,
//! the browser. Called from Rust threads, which attach to the JVM; the
//! object is held as a global reference taken at `start`.
//!
//! The Kotlin side's contract (its methods and their JNI signatures):
//!
//! - `secretAvailable(): Boolean` — a strong biometric is enrolled;
//! - `secretGet(account: String, out: ByteArray): Int` — 0: no key,
//!   1: the key is in `out` (32 bytes), 2: the biometric prompt was not
//!   passed for this step (authentication required);
//! - `secretSet(account: String, key: ByteArray)` and
//!   `secretDelete(account: String)`, which throw on failure;
//! - `clipboardRead(): String?` (throws without focus, Android 10+),
//!   `clipboardWrite(text: String)`, `clipboardWriteSensitive(text: ByteArray)`,
//!   `clipboardClear()`;
//! - `openUrl(url: String)`.
//!
//! Byte arrays that held a key or a password are filled with zeros before
//! they are let go.

use jni::objects::{JByteArray, JObject};
use jni::refs::Global;
use jni::strings::JNIString;
use jni::sys::jbyte;
use jni::{Env, JValue, JavaVM, jni_sig, jni_str};
use secrecy::{ExposeSecret, SecretBox, SecretString};
use shellpp_core::platform::{
    Availability, Clipboard, ExternalLauncher, PlatformError, SecretStore, SecretStoreError,
    UnavailableReason,
};
use zeroize::Zeroizing;

/// The app's `Platform` object.
pub struct KotlinPlatform {
    vm: JavaVM,
    object: Global<JObject<'static>>,
}

impl std::fmt::Debug for KotlinPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KotlinPlatform").finish_non_exhaustive()
    }
}

/// A failed call into the JVM (a Java exception included), without detail:
/// the texts could name accounts.
#[derive(Debug)]
struct CallFailed;

impl From<jni::errors::Error> for CallFailed {
    fn from(_: jni::errors::Error) -> Self {
        Self
    }
}

fn wipe(env: &Env<'_>, array: &JByteArray<'_>, len: usize) {
    let zeros = vec![0 as jbyte; len];
    let _ = array.set_region(env, 0, &zeros);
}

impl KotlinPlatform {
    pub fn new(vm: JavaVM, object: Global<JObject<'static>>) -> Self {
        Self { vm, object }
    }

    fn with_env<T>(
        &self,
        call: impl FnOnce(&mut Env<'_>, &JObject<'static>) -> Result<T, CallFailed>,
    ) -> Result<T, CallFailed> {
        self.vm
            .attach_current_thread(|env| call(env, self.object.as_obj()))
    }

    fn string<'local>(
        env: &mut Env<'local>,
        text: &str,
    ) -> Result<jni::objects::JString<'local>, CallFailed> {
        Ok(jni::objects::JString::from_jni_str(
            env,
            JNIString::new(text),
        )?)
    }
}

impl SecretStore for KotlinPlatform {
    fn availability(&self) -> Availability {
        let available = self.with_env(|env, object| {
            Ok(env
                .call_method(object, jni_str!("secretAvailable"), jni_sig!("()Z"), &[])?
                .z()?)
        });
        match available {
            Ok(true) => Availability::Available,
            _ => Availability::Unavailable(UnavailableReason::Unsupported),
        }
    }

    fn get(&self, account: &str) -> Result<Option<SecretBox<[u8; 32]>>, SecretStoreError> {
        let read = self.with_env(|env, object| {
            let account = Self::string(env, account)?;
            let out = env.new_byte_array(32)?;
            let status = env.call_method(
                object,
                jni_str!("secretGet"),
                jni_sig!("(Ljava/lang/String;[B)I"),
                &[JValue::from(&account), JValue::from(&out)],
            );
            let mut bytes = Zeroizing::new([0 as jbyte; 32]);
            let status = match status.and_then(|value| value.i()) {
                Ok(1) => out.get_region(env, 0, &mut bytes[..]).map(|()| 1),
                other => other,
            };
            wipe(env, &out, 32);
            Ok((status?, bytes))
        });
        match read {
            Ok((0, _)) => Ok(None),
            Ok((1, bytes)) => Ok(Some(SecretBox::init_with_mut(|key: &mut [u8; 32]| {
                for (to, from) in key.iter_mut().zip(bytes.iter()) {
                    *to = *from as u8;
                }
            }))),
            Ok((2, _)) => Err(SecretStoreError::AuthenticationRequired),
            Ok(_) => Err(SecretStoreError::Malformed),
            Err(CallFailed) => Err(SecretStoreError::Backend("keystore".to_owned())),
        }
    }

    fn set(&self, account: &str, secret: &SecretBox<[u8; 32]>) -> Result<(), SecretStoreError> {
        self.with_env(|env, object| {
            let account = Self::string(env, account)?;
            let key = Zeroizing::new(secret.expose_secret().map(|byte| byte as jbyte));
            let array = env.new_byte_array(32)?;
            array.set_region(env, 0, &key[..])?;
            let called = env.call_method(
                object,
                jni_str!("secretSet"),
                jni_sig!("(Ljava/lang/String;[B)V"),
                &[JValue::from(&account), JValue::from(&array)],
            );
            wipe(env, &array, 32);
            called?;
            Ok(())
        })
        .map_err(|CallFailed| SecretStoreError::Backend("keystore".to_owned()))
    }

    fn delete(&self, account: &str) -> Result<(), SecretStoreError> {
        self.with_env(|env, object| {
            let account = Self::string(env, account)?;
            env.call_method(
                object,
                jni_str!("secretDelete"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::from(&account)],
            )?;
            Ok(())
        })
        .map_err(|CallFailed| SecretStoreError::Backend("keystore".to_owned()))
    }
}

impl Clipboard for KotlinPlatform {
    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        self.with_env(|env, object| {
            let text = env
                .call_method(
                    object,
                    jni_str!("clipboardRead"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            if text.is_null() {
                return Ok(None);
            }
            let text = env.cast_local::<jni::objects::JString>(text)?;
            Ok(Some(text.try_to_string(env)?))
        })
        .map_err(|CallFailed| PlatformError::Unavailable)
    }

    fn write_text(&self, text: &str) -> Result<(), PlatformError> {
        self.with_env(|env, object| {
            let text = Self::string(env, text)?;
            env.call_method(
                object,
                jni_str!("clipboardWrite"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::from(&text)],
            )?;
            Ok(())
        })
        .map_err(|CallFailed| PlatformError::Unavailable)
    }

    fn write_sensitive(&self, text: &SecretString) -> Result<(), PlatformError> {
        self.with_env(|env, object| {
            let bytes = text.expose_secret().as_bytes();
            let array = env.byte_array_from_slice(bytes)?;
            let called = env.call_method(
                object,
                jni_str!("clipboardWriteSensitive"),
                jni_sig!("([B)V"),
                &[JValue::from(&array)],
            );
            wipe(env, &array, bytes.len());
            called?;
            Ok(())
        })
        .map_err(|CallFailed| PlatformError::Unavailable)
    }

    fn clear(&self) -> Result<(), PlatformError> {
        self.with_env(|env, object| {
            env.call_method(object, jni_str!("clipboardClear"), jni_sig!("()V"), &[])?;
            Ok(())
        })
        .map_err(|CallFailed| PlatformError::Unavailable)
    }
}

impl ExternalLauncher for KotlinPlatform {
    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        self.with_env(|env, object| {
            let url = Self::string(env, url)?;
            env.call_method(
                object,
                jni_str!("openUrl"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::from(&url)],
            )?;
            Ok(())
        })
        .map_err(|CallFailed| PlatformError::Unavailable)
    }
}
