//! libkbcore.so — puente JNI entre la app Kotlin y el núcleo Rust.
//!
//! Reglas:
//! * Las funciones que bloquean (`connect`, `submitPin`, `nextFrame`,
//!   `nextControl`) deben llamarse desde hilos de fondo de Kotlin.
//! * `touch`/`wheel` NO bloquean: encolan el evento y vuelven enseguida
//!   (se llaman desde el hilo de UI).
//! * Los errores se lanzan como `java.lang.IllegalStateException` con el
//!   mensaje `"<código>|<detalle>"`; Kotlin traduce el código a ES/EN.

pub mod api;

use api::Session;
use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::sys::{jboolean, jfloat, jint, jlong, jobjectArray, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

fn throw(env: &mut JNIEnv, msg: &str) {
    let _ = env.throw_new("java/lang/IllegalStateException", msg);
}

fn jstr(env: &mut JNIEnv, s: &JString) -> String {
    if s.is_null() {
        return String::new();
    }
    env.get_string(s).map(|s| s.into()).unwrap_or_default()
}

fn new_jstring(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn session<'a>(h: jlong) -> Option<&'a Session> {
    if h == 0 {
        None
    } else {
        // SAFETY: `h` es un Box<Session> creado por `connect` y liberado sólo por `close`.
        Some(unsafe { &*(h as *const Session) })
    }
}

#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_generateIdentity(mut env: JNIEnv, _c: JClass) -> jobjectArray {
    match kb_transport::Identity::generate() {
        Ok(id) => {
            let cls = match env.find_class("[B") {
                Ok(c) => c,
                Err(_) => return std::ptr::null_mut(),
            };
            let arr = match env.new_object_array(2, cls, JObject::null()) {
                Ok(a) => a,
                Err(_) => return std::ptr::null_mut(),
            };
            let c = env.byte_array_from_slice(&id.cert_der).unwrap();
            let k = env.byte_array_from_slice(id.key_der()).unwrap();
            let _ = env.set_object_array_element(&arr, 0, c);
            let _ = env.set_object_array_element(&arr, 1, k);
            arr.into_raw()
        }
        Err(e) => {
            throw(&mut env, &format!("io|{e}"));
            std::ptr::null_mut()
        }
    }
}

/// `mode`: 0 = host confiable (huella fijada), 1 = QR (uri), 2 = PIN.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_connect(
    mut env: JNIEnv,
    _c: JClass,
    host: JString,
    port: jint,
    mode: jint,
    pinned_fp: JString,
    qr_uri: JString,
    cert: JByteArray,
    key: JByteArray,
    device_name: JString,
    is_tablet: jboolean,
    screen_w: jint,
    screen_h: jint,
    dpi: jint,
    refresh: jint,
    dec_max_w: jint,
    dec_max_h: jint,
) -> jlong {
    let args = api::ConnectArgs {
        host: jstr(&mut env, &host),
        port: port as u16,
        mode,
        pinned_fp: jstr(&mut env, &pinned_fp),
        qr_uri: jstr(&mut env, &qr_uri),
        cert: env.convert_byte_array(&cert).unwrap_or_default(),
        key: env.convert_byte_array(&key).unwrap_or_default(),
        device_name: jstr(&mut env, &device_name),
        is_tablet: is_tablet == JNI_TRUE,
        screen_w: screen_w as u32,
        screen_h: screen_h as u32,
        dpi: dpi as u32,
        refresh: refresh as u16,
        dec_max_w: dec_max_w as u32,
        dec_max_h: dec_max_h as u32,
    };
    match api::connect(args) {
        Ok(s) => Box::into_raw(Box::new(s)) as jlong,
        Err(e) => {
            throw(&mut env, &e);
            0
        }
    }
}

macro_rules! with_session {
    ($env:ident, $h:ident, $default:expr, |$s:ident| $body:expr) => {
        match session($h) {
            Some($s) => $body,
            None => {
                throw(&mut $env, "closed|sesión cerrada");
                $default
            }
        }
    };
}

#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_stage(mut env: JNIEnv, _c: JClass, h: jlong) -> jstring {
    with_session!(env, h, std::ptr::null_mut(), |s| {
        let j = s.stage_json();
        new_jstring(&mut env, &j)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_hostInfo(mut env: JNIEnv, _c: JClass, h: jlong) -> jstring {
    with_session!(env, h, std::ptr::null_mut(), |s| {
        let j = s.host_json();
        new_jstring(&mut env, &j)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_requestPin(mut env: JNIEnv, _c: JClass, h: jlong) -> jint {
    with_session!(env, h, 0, |s| match s.request_pin() {
        Ok(t) => t as jint,
        Err(e) => {
            throw(&mut env, &e);
            0
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_submitPin(mut env: JNIEnv, _c: JClass, h: jlong, pin: JString) -> jstring {
    let pin = jstr(&mut env, &pin);
    with_session!(env, h, std::ptr::null_mut(), |s| match s.submit_pin(&pin) {
        Ok(j) => new_jstring(&mut env, &j),
        Err(e) => {
            throw(&mut env, &e);
            std::ptr::null_mut()
        }
    })
}

/// Devuelve `cabecera(21 bytes) + H.264`, o `null` si la conexión se cerró.
#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_nextFrame<'a>(env: JNIEnv<'a>, _c: JClass, h: jlong) -> JByteArray<'a> {
    let null = JByteArray::from(JObject::null());
    match session(h).and_then(|s| s.next_frame()) {
        Some(v) => env.byte_array_from_slice(&v).unwrap_or(null),
        None => null,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_nextControl(mut env: JNIEnv, _c: JClass, h: jlong) -> jstring {
    match session(h) {
        Some(s) => {
            let j = s.next_control();
            new_jstring(&mut env, &j)
        }
        None => new_jstring(&mut env, r#"{"type":"closed"}"#),
    }
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_setViewport(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
    view_w: jfloat,
    view_h: jfloat,
    frame_w: jint,
    frame_h: jint,
    zoom: jfloat,
    off_x: jfloat,
    off_y: jfloat,
) {
    if let Some(s) = session(h) {
        s.set_viewport(view_w, view_h, frame_w as u32, frame_h as u32, zoom, off_x, off_y);
    }
}

/// `phase`: 0 down, 1 move, 2 up, 3 cancel. Devuelve si el toque cayó sobre la imagen.
#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_touch(_env: JNIEnv, _c: JClass, h: jlong, phase: jint, x: jfloat, y: jfloat) -> jboolean {
    match session(h) {
        Some(s) if s.touch(phase, x, y) => JNI_TRUE,
        _ => JNI_FALSE,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_wheel(_env: JNIEnv, _c: JClass, h: jlong, x: jfloat, y: jfloat, dy: jint) {
    if let Some(s) = session(h) {
        s.wheel(x, y, dy as i16);
    }
}

#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_requestKeyframe(_env: JNIEnv, _c: JClass, h: jlong) {
    if let Some(s) = session(h) {
        s.send(kb_proto::Message::RequestKeyframe);
    }
}

#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_selectMonitor(mut env: JNIEnv, _c: JClass, h: jlong, id: JString) {
    let id = jstr(&mut env, &id);
    if let Some(s) = session(h) {
        s.send(kb_proto::Message::ModeChangeRequest { mode: kb_proto::Mode::Mirror { monitor_id: id } });
    }
}

#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_monitors(mut env: JNIEnv, _c: JClass, h: jlong) -> jstring {
    let j = session(h).map(|s| s.monitors_json()).unwrap_or_else(|| "[]".into());
    new_jstring(&mut env, &j)
}

#[no_mangle]
pub extern "system" fn Java_com_castrolabs_kbstudioscreen_NativeCore_close(_env: JNIEnv, _c: JClass, h: jlong) {
    if h != 0 {
        // SAFETY: ver `session`. Kotlin garantiza no usar el handle después.
        let s = unsafe { Box::from_raw(h as *mut Session) };
        s.close();
    }
}
