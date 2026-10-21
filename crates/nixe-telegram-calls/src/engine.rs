//! Safe ownership around NTgCalls' pinned C ABI. TDLib owns key exchange.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_void},
    ptr,
};
#[allow(non_camel_case_types, non_upper_case_globals, non_snake_case, dead_code)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/native.rs"));
}
use ffi::*;
// NTgCalls bundles GLib/GObject. Keep its symbols out of GTK's global lookup
// and bind its internal calls to its own bundled dependencies.
fn try_native() -> Result<&'static Native, String> {
    static LIBRARY: std::sync::OnceLock<Result<Native, String>> = std::sync::OnceLock::new();
    LIBRARY
        .get_or_init(|| {
            let mut paths = Vec::new();
            if let Ok(exe) = std::env::current_exe() {
                if let Some(bin) = exe.parent() {
                    paths.push(bin.join("../share/nixe/lib/libntgcalls.so"));
                }
            }
            paths.push(std::path::PathBuf::from(env!("PULSE_NTG_LIBRARY")));
            let mut errors = Vec::new();
            for path in paths {
                if !path.is_file() {
                    continue;
                }
                let result = unsafe {
                    libloading::os::unix::Library::open(Some(&path), libc::RTLD_NOW | libc::RTLD_LOCAL | libc::RTLD_DEEPBIND)
                };
                match result.and_then(|library| unsafe { Native::from_library(library) }) {
                    Ok(library) => return Ok(library),
                    Err(e) => errors.push(e.to_string()),
                }
            }
            Err(format!("The calling engine could not be loaded. Reinstall Pulse. {}", errors.join("; ")))
        })
        .as_ref()
        .map_err(Clone::clone)
}
fn native_api() -> &'static Native {
    try_native().expect("Call entry points validate the native library before use")
}

#[derive(Clone, Debug)]
pub enum EngineEvent {
    BroadcastTime(i64),
    BroadcastPart(i64, i64, i32, i32, i64, bool, i32, u32),
    Signal(i64, Vec<u8>),
    Connection(i64, u32),
    Frame(i64, u32, u16, u16, u32, Vec<u8>),
}
pub struct Engine {
    handle: *mut ntg_instance,
    events: Box<async_channel::Sender<EngineEvent>>,
}
unsafe impl Send for Engine {}
impl Engine {
    pub fn new() -> Result<(Self, async_channel::Receiver<EngineEvent>), String> {
        try_native()?;
        unsafe {
            let handle = native_api().ntg_instance_create();
            if handle.is_null() {
                return Err("Could not initialize call engine".into());
            }
            let (tx, rx) = async_channel::bounded(64);
            let mut engine = Self { handle, events: Box::new(tx) };
            let data = (&mut *engine.events as *mut async_channel::Sender<EngineEvent>).cast();
            check(native_api().ntg_on_signaling_data_callback(handle, Some(signal), data))?;
            check(native_api().ntg_on_connection_change_callback(handle, Some(connection), data))?;
            check(native_api().ntg_on_frames_callback(handle, Some(frames), data))?;
            check(native_api().ntg_on_request_broadcast_timestamp_callback(handle, Some(broadcast_time), data))?;
            check(native_api().ntg_on_request_broadcast_part_callback(handle, Some(broadcast_part), data))?;
            Ok((engine, rx))
        }
    }
    pub fn protocol() -> Result<Value, String> {
        try_native()?;
        unsafe {
            let mut p: ntg_protocol = std::mem::zeroed();
            check(native_api().ntg_get_protocol(&mut p))?;
            let versions = std::slice::from_raw_parts(p.library_versions, p.library_versions_len)
                .iter()
                .map(|s| CStr::from_ptr(*s).to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            let val = json!({"@type":"callProtocol","min_layer":p.min_layer,"max_layer":p.max_layer,"udp_p2p":p.udp_p2p,"udp_reflector":p.udp_reflector,"library_versions":versions});
            native_api().ntg_protocol_free(&mut p);
            Ok(val)
        }
    }
    pub fn broadcast_time(&mut self, chat: i64, timestamp: i64) -> Result<(), String> {
        unsafe { check(native_api().ntg_send_broadcast_timestamp(self.handle, chat, timestamp)) }
    }
    pub fn broadcast_part(
        &mut self,
        chat: i64,
        segment: i64,
        part: i32,
        status: u32,
        quality_update: bool,
        data: &[u8],
    ) -> Result<(), String> {
        unsafe {
            check(native_api().ntg_send_broadcast_part(
                self.handle,
                chat,
                segment,
                part,
                status,
                quality_update,
                data.as_ptr(),
                data.len(),
            ))
        }
    }
    pub fn create_group(&mut self, chat: i64) -> Result<String, String> {
        unsafe {
            let mut payload = ptr::null_mut();
            check(native_api().ntg_create_call(self.handle, chat, &mut payload))?;
            if payload.is_null() {
                return Err("The group call engine returned no join payload.".into());
            }
            let value = CStr::from_ptr(payload).to_string_lossy().into_owned();
            native_api().ntg_string_free(payload);
            Ok(value)
        }
    }
    pub fn add_group_video(&mut self, chat: i64, user: i64, info: &Value) -> Result<u32, String> {
        let endpoint = cstring(info["endpoint_id"].as_str().ok_or("Missing video endpoint.")?);
        let groups = info["source_groups"].as_array().ok_or("Missing video source groups.")?;
        let semantics = groups.iter().map(|g| cstring(g["semantics"].as_str().unwrap_or(""))).collect::<Vec<_>>();
        let mut ids = groups
            .iter()
            .map(|g| {
                g["source_ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_i64)
                    .map(|id| id as i32 as u32)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let native = semantics
            .iter()
            .zip(ids.iter_mut())
            .map(|(semantics, ids)| ntg_ssrc_group {
                semantics: semantics.as_ptr().cast_mut(),
                ssrcs: ids.as_mut_ptr(),
                ssrcs_len: ids.len(),
            })
            .collect::<Vec<_>>();
        let mut source = 0;
        unsafe {
            check(native_api().ntg_add_incoming_video(
                self.handle,
                chat,
                user,
                endpoint.as_ptr(),
                native.as_ptr(),
                native.len(),
                &mut source,
            ))?;
        }
        Ok(source)
    }
    pub fn remove_group_video(&mut self, chat: i64, endpoint: &str) -> Result<(), String> {
        let endpoint = cstring(endpoint);
        let mut removed = false;
        unsafe { check(native_api().ntg_remove_incoming_video(self.handle, chat, endpoint.as_ptr(), &mut removed)) }
    }
    pub fn create_presentation(&mut self, chat: i64) -> Result<String, String> {
        unsafe {
            let mut out = ptr::null_mut();
            check(native_api().ntg_init_presentation(self.handle, chat, &mut out))?;
            if out.is_null() {
                return Err("No screen-sharing payload.".into());
            }
            let result = CStr::from_ptr(out).to_string_lossy().into_owned();
            native_api().ntg_string_free(out);
            Ok(result)
        }
    }
    pub fn connect_presentation(&mut self, chat: i64, payload: &str) -> Result<(), String> {
        let payload = cstring(payload);
        unsafe { check(native_api().ntg_connect(self.handle, chat, payload.as_ptr(), true)) }
    }
    pub fn stop_presentation(&mut self, chat: i64) {
        unsafe {
            native_api().ntg_stop_presentation(self.handle, chat);
        }
    }
    pub fn connect_group(&mut self, chat: i64, payload: &str) -> Result<(), String> {
        let payload = cstring(payload);
        unsafe { check(native_api().ntg_connect(self.handle, chat, payload.as_ptr(), false)) }
    }
    pub fn create(&mut self, user: i64) -> Result<(), String> {
        unsafe { check(native_api().ntg_create_p2p_call(self.handle, user)) }
    }
    pub fn connect(&mut self, user: i64, ready: &Value, outgoing: bool) -> Result<(), String> {
        unsafe {
            let key = STANDARD.decode(ready["encryption_key"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
            check(native_api().ntg_skip_exchange(self.handle, user, key.as_ptr(), key.len(), outgoing))?;
            let mut strings = Vec::<CString>::new();
            let mut peers = Vec::<Vec<u8>>::new();
            let mut servers = Vec::new();
            for s in ready["servers"].as_array().into_iter().flatten() {
                let t = &s["type"];
                let ip4 = cstring(s["ip_address"].as_str().unwrap_or(""));
                let ip6 = cstring(s["ipv6_address"].as_str().unwrap_or(""));
                let user_name = cstring(t["username"].as_str().unwrap_or(""));
                let pass = cstring(t["password"].as_str().unwrap_or(""));
                let peer = STANDARD.decode(t["peer_tag"].as_str().unwrap_or("")).unwrap_or_default();
                servers.push(ntg_rtc_server {
                    id: s["id"].as_i64().or_else(|| s["id"].as_str().and_then(|s| s.parse().ok())).unwrap_or(0) as u64,
                    ipv4: ip4.as_ptr().cast_mut(),
                    ipv6: ip6.as_ptr().cast_mut(),
                    port: s["port"].as_u64().unwrap_or(0) as u16,
                    username: user_name.as_ptr().cast_mut(),
                    password: pass.as_ptr().cast_mut(),
                    turn: t["supports_turn"].as_bool().unwrap_or(false),
                    stun: t["supports_stun"].as_bool().unwrap_or(false),
                    tcp: t["is_tcp"].as_bool().unwrap_or(false),
                    peer_tag: peer.as_ptr().cast_mut(),
                    peer_tag_len: peer.len(),
                });
                strings.extend([ip4, ip6, user_name, pass]);
                peers.push(peer);
            }
            let versions = ready["protocol"]["library_versions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(cstring)
                .collect::<Vec<_>>();
            let version_ptrs = versions.iter().map(|s| s.as_ptr()).collect::<Vec<_>>();
            let parameters = cstring(ready["custom_parameters"].as_str().unwrap_or(""));
            check(native_api().ntg_connect_p2p(
                self.handle,
                user,
                servers.as_ptr(),
                servers.len(),
                version_ptrs.as_ptr().cast_mut(),
                version_ptrs.len(),
                ready["allow_p2p"].as_bool().unwrap_or(false),
                if ready["custom_parameters"].as_str().unwrap_or("").is_empty() { ptr::null() } else { parameters.as_ptr() },
            ))
        }
    }
    pub fn signal(&mut self, user: i64, data: &[u8]) -> Result<(), String> {
        unsafe { check(native_api().ntg_send_signaling_data(self.handle, user, data.as_ptr(), data.len())) }
    }
    pub fn stop(&mut self, user: i64) {
        unsafe {
            native_api().ntg_stop(self.handle, user);
        }
    }
    pub fn mute(&mut self, user: i64, on: bool) -> Result<(), String> {
        unsafe {
            let mut result = false;
            check(if on {
                native_api().ntg_mute(self.handle, user, &mut result)
            } else {
                native_api().ntg_unmute(self.handle, user, &mut result)
            })
        }
    }
    pub fn devices() -> Result<Value, String> {
        try_native()?;
        unsafe {
            let mut d: ntg_media_devices = std::mem::zeroed();
            check(native_api().ntg_get_media_devices(&mut d))?;
            fn items(p: *mut ntg_device_info, n: usize) -> Vec<Value> {
                if n == 0 {
                    return vec![];
                }
                unsafe {
                    std::slice::from_raw_parts(p,n).iter().map(|d|json!({"name":CStr::from_ptr(d.name).to_string_lossy(),"metadata":CStr::from_ptr(d.metadata).to_string_lossy()})).collect()
                }
            }
            let val = json!({"microphone":items(d.microphone,d.microphone_len),"speaker":items(d.speaker,d.speaker_len),"camera":items(d.camera,d.camera_len),"screen":items(d.screen,d.screen_len)});
            native_api().ntg_media_devices_free(&mut d);
            Ok(val)
        }
    }
    pub fn media(
        &mut self,
        user: i64,
        microphone: &str,
        speaker: &str,
        camera: Option<&str>,
        screen: bool,
    ) -> Result<(), String> {
        unsafe {
            let mic = cstring(microphone);
            let sp = cstring(speaker);
            let cam = cstring(camera.unwrap_or(""));
            let mut mic_desc = ntg_audio_description {
                media_source: NTG_MEDIA_SOURCE_DEVICE,
                sample_rate: 48000,
                channel_count: 1,
                input: mic.as_ptr().cast_mut(),
                keep_open: true,
            };
            let mut speaker_desc = ntg_audio_description {
                media_source: NTG_MEDIA_SOURCE_DEVICE,
                sample_rate: 48000,
                channel_count: 2,
                input: sp.as_ptr().cast_mut(),
                keep_open: true,
            };
            let mut camera_desc = ntg_video_description {
                media_source: NTG_MEDIA_SOURCE_DEVICE,
                width: 640,
                height: 480,
                fps: 30,
                input: cam.as_ptr().cast_mut(),
                keep_open: true,
            };
            let mut screen_desc = ntg_video_description {
                media_source: NTG_MEDIA_SOURCE_EXTERNAL,
                width: 640,
                height: 360,
                fps: 15,
                input: ptr::null_mut(),
                keep_open: true,
            };
            let capture = ntg_media_description {
                microphone: if microphone.is_empty() { ptr::null_mut() } else { &mut mic_desc },
                speaker: ptr::null_mut(),
                camera: if camera.is_some() && !screen { &mut camera_desc } else { ptr::null_mut() },
                screen: if screen { &mut screen_desc } else { ptr::null_mut() },
            };
            check(native_api().ntg_set_stream_sources(self.handle, user, NTG_STREAM_MODE_CAPTURE, capture))?;
            let mut remote_video = ntg_video_description {
                media_source: NTG_MEDIA_SOURCE_EXTERNAL,
                width: 0,
                height: 0,
                fps: 30,
                input: ptr::null_mut(),
                keep_open: true,
            };
            let playback = ntg_media_description {
                microphone: ptr::null_mut(),
                speaker: &mut speaker_desc,
                camera: &mut remote_video,
                screen: ptr::null_mut(),
            };
            check(native_api().ntg_set_stream_sources(self.handle, user, NTG_STREAM_MODE_PLAYBACK, playback))
        }
    }
    pub fn screen(&mut self, user: i64, frame: &[u8], width: u16, height: u16) -> Result<(), String> {
        unsafe {
            let data =
                ntg_frame_data { absolute_capture_timestamp_ms: 0, rotation: NTG_VIDEO_ROTATION_VIDEO_ROTATION_0, width, height };
            check(native_api().ntg_send_external_frame(
                self.handle,
                user,
                NTG_STREAM_DEVICE_SCREEN,
                frame.as_ptr(),
                frame.len(),
                data,
            ))
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { native_api().ntg_instance_destroy(self.handle) }
    }
}
fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap()
}
fn check(result: ntg_result) -> Result<(), String> {
    if result == NTG_OK {
        Ok(())
    } else {
        unsafe {
            let error = native_api().ntg_last_error();
            Err(if error.is_null() {
                format!("Call engine error {result}")
            } else {
                CStr::from_ptr(error).to_string_lossy().into_owned()
            })
        }
    }
}
unsafe extern "C" fn signal(_: *mut ntg_instance, user: i64, data: *const u8, len: usize, ctx: *mut c_void) {
    unsafe {
        if !data.is_null() && !ctx.is_null() {
            let tx = &*ctx.cast::<async_channel::Sender<EngineEvent>>();
            let _ = tx.send_blocking(EngineEvent::Signal(user, std::slice::from_raw_parts(data, len).to_vec()));
        }
    }
}
unsafe extern "C" fn connection(_: *mut ntg_instance, user: i64, info: ntg_connection_info, ctx: *mut c_void) {
    unsafe {
        if !ctx.is_null() {
            let _ = (&*ctx.cast::<async_channel::Sender<EngineEvent>>()).try_send(EngineEvent::Connection(user, info.state));
        }
    }
}
unsafe extern "C" fn frames(
    _: *mut ntg_instance,
    user: i64,
    mode: ntg_stream_mode,
    device: ntg_stream_device,
    data: *const ntg_frame,
    len: usize,
    ctx: *mut c_void,
) {
    unsafe {
        if mode == NTG_STREAM_MODE_PLAYBACK
            && (device == NTG_STREAM_DEVICE_CAMERA || device == NTG_STREAM_DEVICE_SCREEN)
            && !data.is_null()
            && !ctx.is_null()
        {
            for frame in std::slice::from_raw_parts(data, len) {
                if !frame.data.is_null() && frame.data_len <= 1920 * 1080 * 4 {
                    let _ = (&*ctx.cast::<async_channel::Sender<EngineEvent>>()).try_send(EngineEvent::Frame(
                        user,
                        frame.ssrc as u32,
                        frame.frame_data.width,
                        frame.frame_data.height,
                        frame.frame_data.rotation,
                        std::slice::from_raw_parts(frame.data, frame.data_len).to_vec(),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    #[test]
    fn real_native_engine_protocol_and_lifecycle() {
        let protocol = Engine::protocol().unwrap();
        assert!(protocol["library_versions"].as_array().is_some_and(|v| !v.is_empty()));
        assert!(protocol["max_layer"].as_i64().unwrap() >= protocol["min_layer"].as_i64().unwrap());
        let devices = Engine::devices().unwrap();
        assert!(devices["microphone"].is_array());
        assert!(devices["speaker"].is_array());
        let (mut engine, _events) = Engine::new().unwrap();
        engine.create(123456789).unwrap();
        engine.stop(123456789);
    }
}

#[cfg(test)]
mod transport_regression {
    use super::*;
    #[test]
    fn local_signaling_loopback() {
        let protocol = Engine::protocol().unwrap();
        let ready = json!({"encryption_key":STANDARD.encode([73u8;256]),"protocol":protocol,"servers":[],"allow_p2p":true,"custom_parameters":""});
        let (mut a, arx) = Engine::new().unwrap();
        let (mut b, brx) = Engine::new().unwrap();
        a.create(11).unwrap();
        b.create(22).unwrap();
        a.connect(11, &ready, true).unwrap();
        b.connect(22, &ready, false).unwrap();
        let start = std::time::Instant::now();
        let (mut ac, mut bc) = (false, false);
        while start.elapsed() < std::time::Duration::from_secs(5) && !(ac && bc) {
            while let Ok(event) = arx.try_recv() {
                match event {
                    EngineEvent::Signal(_, data) => b.signal(22, &data).unwrap(),
                    EngineEvent::Connection(_, state) => ac |= state == 1,
                    _ => {}
                }
            }
            while let Ok(event) = brx.try_recv() {
                match event {
                    EngineEvent::Signal(_, data) => a.signal(11, &data).unwrap(),
                    EngineEvent::Connection(_, state) => bc |= state == 1,
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        a.stop(11);
        b.stop(22);
        assert!(ac && bc, "Native call peers did not connect locally");
    }
}

unsafe extern "C" fn broadcast_time(_: *mut ntg_instance, chat: i64, data: *mut c_void) {
    if !data.is_null() {
        let tx = unsafe { &*(data as *const async_channel::Sender<EngineEvent>) };
        let _ = tx.try_send(EngineEvent::BroadcastTime(chat));
    }
}
unsafe extern "C" fn broadcast_part(_: *mut ntg_instance, chat: i64, request: ntg_segment_part_request, data: *mut c_void) {
    if !data.is_null() {
        let tx = unsafe { &*(data as *const async_channel::Sender<EngineEvent>) };
        let _ = tx.try_send(EngineEvent::BroadcastPart(
            chat,
            request.segment_id,
            request.part_id,
            request.limit,
            request.timestamp,
            request.quality_update,
            request.channel_id,
            request.quality,
        ));
    }
}

#[cfg(test)]
mod isolation_tests {
    use super::*;
    #[test]
    fn protocol_load_keeps_bundled_glib_private() {
        let protocol = Engine::protocol().unwrap();
        assert!(!protocol["library_versions"].as_array().unwrap().is_empty());
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ntg_instance_create".as_ptr()) };
        assert!(symbol.is_null(), "The calling engine must not export its bundled GLib into GTK's global symbol scope");
    }
}
