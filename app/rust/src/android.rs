//! Android start-up: hands the Java VM and app context to the crates that
//! need them (cpal's AAudio output asks Android for its audio settings).
use std::ffi::c_void;
use std::sync::Once;

use jni_sys::{jobject, JNIEnv, JavaVM, JNI_OK};

static INIT: Once = Once::new();

/// Called once from `MainActivity.onCreate`.
///
/// # Safety
/// Called by the JVM with a valid environment and context object.
#[no_mangle]
pub unsafe extern "system" fn Java_io_github_aimatochysia_atropos_MainActivity_initNative(
    env: *mut JNIEnv,
    _this: jobject,
    context: jobject,
) {
    INIT.call_once(|| {
        if env.is_null() || context.is_null() {
            return;
        }
        let fns = &**env;
        let (Some(get_vm), Some(new_global)) = (fns.GetJavaVM, fns.NewGlobalRef) else {
            return;
        };
        let mut vm: *mut JavaVM = std::ptr::null_mut();
        if get_vm(env, &mut vm) != JNI_OK || vm.is_null() {
            return;
        }
        // Kept for the life of the process.
        let global = new_global(env, context);
        if global.is_null() {
            return;
        }
        ndk_context::initialize_android_context(vm as *mut c_void, global as *mut c_void);
    });
}
