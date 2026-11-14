//! JNI: the entry `NshView.runRustApp` calls, and the view's end of the
//! byte channel (`ByteChannel.java`), as thin wrappers of
//! `moshpit_stream::view`.
//!
//! Output reaches the view in one copy: [`read`] moves the queue's bytes
//! straight into the Java array (pinned with `GetPrimitiveArrayCritical`).
//! Input is one copy too: [`write`] appends the pinned array's bytes to the
//! core's input queue, synchronously, on the caller's thread.

use moshpit_stream::view;
use nsh::jni::{
    JNIEnv,
    objects::{JByteArray, JClass, JObject, ReleaseMode},
    sys::{jboolean, jint, jlong, jlongArray},
};

use crate::{alog, core, ui};

/// `Native.runRust` of NSH's runtime, which `NshView.runRustApp` calls on
/// the UI thread: hands NSH the core's Tokio runtime for this view, then
/// mounts the root. What `nsh::main!` defines, plus `set_runtime`.
#[unsafe(no_mangle)]
extern "system" fn Java_com_longbridge_nsh_Native_runRust(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    handle: jlong,
) {
    // SAFETY: Java passes its view's live handle, on the view's UI thread,
    // before the view loads an app (this call is the load).
    unsafe { nsh::set_runtime(handle, Some(core::RUNTIME.handle().clone())) };
    nsh::run_root(handle, ui::root());
}

fn id(stream: jlong) -> u64 {
    stream as u64
}

/// `ByteChannel.attach(stream, transport)`: the core wakes the view by
/// calling `transport.wake()`, on whatever thread pushed.
#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_spike_ByteChannel_attach(
    env: JNIEnv<'_>,
    _: JClass<'_>,
    stream: jlong,
    transport: JObject<'_>,
) -> jboolean {
    let (Ok(vm), Ok(target)) = (env.get_java_vm(), env.new_global_ref(&transport)) else {
        return 0;
    };
    let attached = view::attach(id(stream), move || {
        // The core's threads are attached once and stay attached.
        match vm.attach_current_thread_permanently() {
            Ok(mut env) => {
                if env.call_method(&target, "wake", "()V", &[]).is_err() {
                    let _ = env.exception_describe();
                    let _ = env.exception_clear();
                }
            }
            Err(err) => alog::warn(format!("wake: cannot attach the thread: {err}")),
        }
    });
    u8::from(attached)
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_spike_ByteChannel_detach(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    stream: jlong,
) {
    view::detach(id(stream));
}

/// `ByteChannel.read(stream, buffer)`: up to `buffer.length` bytes, oldest
/// first; how many.
#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_spike_ByteChannel_read(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
    stream: jlong,
    buffer: JByteArray<'_>,
) -> jint {
    // SAFETY: nothing calls into the VM while the array is pinned; `read`
    // only takes the queue's lock and copies.
    let Ok(mut pinned) =
        (unsafe { env.get_array_elements_critical(&buffer, ReleaseMode::CopyBack) })
    else {
        return 0;
    };
    // SAFETY: `jbyte` is `i8`, of `u8`'s size and alignment.
    let bytes =
        unsafe { std::slice::from_raw_parts_mut(pinned.as_mut_ptr().cast::<u8>(), pinned.len()) };
    view::read(id(stream), bytes) as jint
}

/// `ByteChannel.write(stream, data, offset, count)`: appends the bytes to
/// the core's input before this returns.
#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_spike_ByteChannel_write(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
    stream: jlong,
    data: JByteArray<'_>,
    offset: jint,
    count: jint,
) -> jboolean {
    let Ok(len) = env.get_array_length(&data) else {
        return 0;
    };
    let (Ok(offset), Ok(count)) = (usize::try_from(offset), usize::try_from(count)) else {
        return 0;
    };
    if offset
        .checked_add(count)
        .is_none_or(|end| end > len as usize)
    {
        return 0;
    }
    // SAFETY: as in `read`; `write` only takes the input's lock and copies.
    let Ok(pinned) = (unsafe { env.get_array_elements_critical(&data, ReleaseMode::NoCopyBack) })
    else {
        return 0;
    };
    // SAFETY: as in `read`.
    let bytes = unsafe { std::slice::from_raw_parts(pinned.as_ptr().cast::<u8>(), pinned.len()) };
    u8::from(view::write(id(stream), &bytes[offset..offset + count]))
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_spike_ByteChannel_resize(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    stream: jlong,
    columns: jint,
    rows: jint,
) -> jboolean {
    let clamp = |n: jint| n.clamp(0, i32::from(u16::MAX)) as u16;
    u8::from(view::resize(id(stream), clamp(columns), clamp(rows)))
}

/// `long[]`: pushes, pushed, read, queued, peak queued, wakes, drains; null
/// if the stream is gone.
#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_spike_ByteChannel_stats(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
    stream: jlong,
) -> jlongArray {
    let Some(s) = view::stats(id(stream)) else {
        return std::ptr::null_mut();
    };
    let values = [
        s.pushes,
        s.pushed,
        s.read,
        s.queued,
        s.peak_queued,
        s.wakes,
        s.drains,
    ]
    .map(|v| v as jlong);
    long_array(&mut env, &values)
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_spike_ByteChannel_burst(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    total: jlong,
    seed: jlong,
) {
    core::burst(total.max(0) as u64, seed as u64);
}

/// `long[]`: length, hash, chunks of the burst, once its last chunk is
/// hashed; null before.
#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_spike_ByteChannel_burstExpected(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
) -> jlongArray {
    let Some((length, hash, chunks)) = *core::BURST.lock().unwrap_or_else(|e| e.into_inner())
    else {
        return std::ptr::null_mut();
    };
    long_array(&mut env, &[length as jlong, hash as jlong, chunks as jlong])
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_spike_ByteChannel_lines(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    count: jint,
) {
    core::lines(count.max(0) as u32);
}

fn long_array(env: &mut JNIEnv<'_>, values: &[jlong]) -> jlongArray {
    let Ok(array) = env.new_long_array(values.len() as i32) else {
        return std::ptr::null_mut();
    };
    if env.set_long_array_region(&array, 0, values).is_err() {
        return std::ptr::null_mut();
    }
    array.into_raw()
}
