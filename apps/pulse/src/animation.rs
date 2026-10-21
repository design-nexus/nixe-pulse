//! Bounded vector animation using the installed Telegram tlottie renderer.
use gtk::{gdk, glib, prelude::*};
use std::{ffi::c_void, io::Read};
#[link(name = "tlottie")]
unsafe extern "C" {
    fn tlottie_new(data: *const u8, len: usize) -> *mut c_void;
    fn tlottie_drop(renderer: *mut c_void);
    fn tlottie_frame_count(renderer: *const c_void) -> u32;
    fn tlottie_frame_rate(renderer: *const c_void) -> f32;
    fn tlottie_render(renderer: *mut c_void, frame: f32, width: u32, height: u32, out: *mut u32, len: usize, aa: u32) -> i32;
}
#[cfg(test)]
thread_local! { static LIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
struct Animation(*mut c_void);
impl Animation {
    // Borrow the owner, rather than capturing its raw pointer in a timer. Rust's
    // disjoint closure capture would otherwise drop the renderer before frame 1.
    fn render(&self, frame: u32, pixels: &mut [u32]) -> bool {
        unsafe { tlottie_render(self.0, frame as f32, 192, 192, pixels.as_mut_ptr(), pixels.len(), 1) == 0 }
    }
}
impl Drop for Animation {
    fn drop(&mut self) {
        #[cfg(test)]
        LIVE.with(|n| n.set(n.get() - 1));
        unsafe { tlottie_drop(self.0) }
    }
}
fn open_animation(path: &str) -> Result<(Animation, u32, f32), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    flate2::read::GzDecoder::new(file).take(4 * 1024 * 1024 + 1).read_to_end(&mut data).map_err(|e| e.to_string())?;
    if data.len() > 4 * 1024 * 1024 {
        return Err("Vector animation is too large".into());
    }
    let ptr = unsafe { tlottie_new(data.as_ptr(), data.len()) };
    if ptr.is_null() {
        return Err("Invalid vector animation".into());
    }
    let anim = Animation(ptr);
    #[cfg(test)]
    LIVE.with(|n| n.set(n.get() + 1));
    let count = unsafe { tlottie_frame_count(ptr) }.max(1);
    let fps = unsafe { tlottie_frame_rate(ptr) };
    if !fps.is_finite() || fps <= 0.0 {
        return Err("Invalid animation frame rate".into());
    }
    let fps = fps.clamp(1.0, 60.0);
    Ok((anim, count, fps))
}
pub fn still(picture: &gtk::Picture, path: &str, repaint: Option<gdk::RGBA>) -> bool {
    let Ok((anim, _, _)) = open_animation(path) else { return false };
    let mut pixels = vec![0u32; 192 * 192];
    if !anim.render(0, &mut pixels) {
        return false;
    }
    if let Some(color) = repaint {
        for pixel in &mut pixels {
            let alpha = pixel.to_ne_bytes()[3];
            *pixel = u32::from_ne_bytes([
                (color.red() * alpha as f32).round() as u8,
                (color.green() * alpha as f32).round() as u8,
                (color.blue() * alpha as f32).round() as u8,
                alpha,
            ]);
        }
    }
    set_frame(picture, pixels);
    true
}
pub fn play(picture: &gtk::Picture, path: &str) -> bool {
    if crate::prefs::get().reduce_motion{return still(picture,path,None)}
    let result = open_animation(path);
    let (anim, count, fps) = match result {
        Ok(v) => v,
        Err(e) => {
            crate::window::show_status(&e);
            return false;
        }
    };
    let name = format!("animation-{}", uuid::Uuid::new_v4());
    picture.set_widget_name(&name);
    let weak = picture.downgrade();
    let mut pixels = vec![0u32; 192 * 192];
    if !anim.render(0, &mut pixels) {
        return false;
    }
    set_frame(picture, pixels);
    let mut frame = 1 % count;
    let mut unmapped = 0;
    glib::timeout_add_local(std::time::Duration::from_secs_f32(1.0 / fps), move || {
        let Some(p) = weak.upgrade() else { return glib::ControlFlow::Break };
        if p.widget_name() != name {
            return glib::ControlFlow::Break;
        }
        if !p.is_mapped() {
            unmapped += 1;
            if unmapped > 10 {
                return glib::ControlFlow::Break;
            }
            return glib::ControlFlow::Continue;
        }
        unmapped = 0;
        let mut pixels = vec![0u32; 192 * 192];
        if !anim.render(frame, &mut pixels) {
            return glib::ControlFlow::Break;
        }
        set_frame(&p, pixels);
        frame = (frame + 1) % count;
        glib::ControlFlow::Continue
    });
    true
}
fn set_frame(picture: &gtk::Picture, pixels: Vec<u32>) {
    let data = pixels.into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
    let texture =
        gdk::MemoryTexture::new(192, 192, gdk::MemoryFormat::R8g8b8a8Premultiplied, &glib::Bytes::from_owned(data), 192 * 4);
    picture.set_paintable(Some(&texture));
}

#[cfg(test)]
pub fn check_timer_lifetime() {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!("pulse-animation-{}.tgs", uuid::Uuid::new_v4()));
    let file = std::fs::File::create(&path).unwrap();
    let mut gzip = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    gzip.write_all(br#"{"v":"5.7.4","fr":30,"ip":0,"op":60,"w":64,"h":64,"layers":[{"ty":4,"ind":1,"ip":0,"op":60,"st":0,"ks":{"o":{"a":0,"k":100},"r":{"a":0,"k":0},"p":{"a":0,"k":[32,32,0]},"a":{"a":0,"k":[0,0,0]},"s":{"a":0,"k":[100,100,100]}},"shapes":[{"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[40,40]}},{"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100},"r":1}]}]}"#).unwrap();
    gzip.finish().unwrap();
    let baseline = LIVE.with(|n| n.get());
    let preview = gtk::Picture::new();
    assert!(still(&preview, path.to_str().unwrap(), Some(gdk::RGBA::WHITE)));
    assert!(preview.paintable().is_some());
    assert_eq!(LIVE.with(|n| n.get()), baseline, "Static topic previews release the native renderer immediately");
    let picture = gtk::Picture::new();
    assert!(play(&picture, path.to_str().unwrap()));
    assert!(picture.paintable().is_some(), "First animation frame is visible immediately");
    assert_eq!(LIVE.with(|n| n.get()), baseline + 1, "Timer must own its renderer after play returns");
    let window = gtk::Window::builder().child(&picture).build();
    window.present();
    let loop_ = glib::MainLoop::new(None, false);
    let done = loop_.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || done.quit());
    loop_.run();
    assert_eq!(LIVE.with(|n| n.get()), baseline + 1);
    window.close();
    window.set_child(gtk::Widget::NONE);
    drop(picture);
    let done = loop_.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || done.quit());
    loop_.run();
    assert_eq!(LIVE.with(|n| n.get()), baseline, "Removed widgets release their renderer");
    std::fs::remove_file(path).unwrap();
}
