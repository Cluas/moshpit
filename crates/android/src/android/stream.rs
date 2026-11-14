//! The view's end of a pane's byte channel (`com.cluas.moshpit.term.
//! ByteChannel`), as thin JNI wrappers of `moshpit_stream::view`: the hot
//! path, no JSON, no polling (`docs/nsh/DESIGN.md` §3.3; gate G3).
//!
//! - Output reaches the view in one copy: `read` moves the queue's bytes
//!   straight into the Java array, pinned with `GetPrimitiveArrayCritical`.
//!   It stops at a mark (what the view does between two bytes): the mark
//!   is handed to the controller (off the main thread) and the read says a
//!   mark came, so the view reads on.
//! - Input is one copy too: `write` appends the pinned array's bytes to
//!   the core's input queue, synchronously, on the caller's thread.
//! - The core wakes the view by calling `transport.wake()` on whatever
//!   thread pushed, at most once until the view has drained the queue.

use moshpit_stream::view;
use nsh::jni::{
    JNIEnv,
    objects::{JByteArray, JClass, JObject, ReleaseMode},
    sys::{jboolean, jint, jlong, jlongArray},
};

fn id(stream: jlong) -> u64 {
    stream as u64
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_term_ByteChannel_attach(
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
            Err(err) => log::warn!(target: "moshpit", "wake: cannot attach the thread: {err}"),
        }
    });
    u8::from(attached)
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_term_ByteChannel_detach(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    stream: jlong,
) {
    view::detach(id(stream));
}

/// `ByteChannel.read(stream, buffer)`: up to `buffer.length` bytes, oldest
/// first, stopping at a mark. `n >= 0`: `n` bytes and no mark, drained when
/// `n < buffer.length`. `-(n + 1)`: `n` bytes, then a mark (handled here):
/// not drained, read again.
#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_term_ByteChannel_read(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
    stream: jlong,
    buffer: JByteArray<'_>,
) -> jint {
    let (n, mark) = {
        // SAFETY: nothing calls into the VM while the array is pinned;
        // `read_marked` only takes the queue's lock and copies.
        let Ok(mut pinned) =
            (unsafe { env.get_array_elements_critical(&buffer, ReleaseMode::CopyBack) })
        else {
            return 0;
        };
        // SAFETY: `jbyte` is `i8`, of `u8`'s size and alignment.
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(pinned.as_mut_ptr().cast::<u8>(), pinned.len())
        };
        view::read_marked(id(stream), bytes)
    };
    match mark {
        None => n as jint,
        Some(mark) => {
            super::view_mark(id(stream), mark);
            -(n as jint) - 1
        }
    }
}

/// `ByteChannel.write(stream, data, offset, count)`: appends the bytes to
/// the core's input before this returns.
#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_term_ByteChannel_write(
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
extern "system" fn Java_com_cluas_moshpit_term_ByteChannel_resize(
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
extern "system" fn Java_com_cluas_moshpit_term_ByteChannel_stats(
    env: JNIEnv<'_>,
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
    let Ok(array) = env.new_long_array(values.len() as i32) else {
        return std::ptr::null_mut();
    };
    if env.set_long_array_region(&array, 0, &values).is_err() {
        return std::ptr::null_mut();
    }
    array.into_raw()
}
