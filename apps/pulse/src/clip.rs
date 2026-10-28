//! Looping video stickers, GIFs and animated custom emoji.
//!
//! GTK's GStreamer video backend aborts the process when many players are created and
//! torn down quickly, which a chat full of stickers does. Clips are instead decoded once
//! by an ffmpeg child process into frames at display size, shared by every picture that
//! shows the same clip, painted from one timer, and released once scrolled out of view.
use gtk::{gdk, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    rc::{Rc, Weak},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};

const FPS: u32 = 30;
const MAX_SECONDS: u32 = 10;
/// Decoded bytes one clip may hold. Over this a clip drops to half rate, then stops early.
const CLIP_BUDGET: usize = 24 * 1024 * 1024;
/// Decoded bytes kept for recently shown clips, so scrolling back or reopening a chat
/// doesn't decode them again. Clips on screen stay alive past this.
const RECENT_BUDGET: usize = 96 * 1024 * 1024;
const DECODERS: usize = 3;
const TICK: Duration = Duration::from_millis(16);
const RELEASE_AFTER: Duration = Duration::from_secs(2);

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Key {
    path: String,
    width: u32,
    height: u32,
    still: bool,
}

struct Decoded {
    width: usize,
    height: usize,
    frames: Vec<Vec<u8>>,
    frame_time: Duration,
}

struct Clip {
    frames: Vec<gdk::Texture>,
    frame_time: Duration,
    bytes: usize,
}

struct Player {
    picture: glib::WeakRef<gtk::Picture>,
    key: Key,
    clip: Option<Rc<Clip>>,
    shown: usize,
    hidden_since: Option<Instant>,
}

thread_local! {
    static PLAYERS: RefCell<Vec<Player>> = const { RefCell::new(Vec::new()) };
    static CLIPS: RefCell<HashMap<Key, Weak<Clip>>> = RefCell::new(HashMap::new());
    static RECENT: RefCell<VecDeque<Rc<Clip>>> = const { RefCell::new(VecDeque::new()) };
    static PENDING: RefCell<HashSet<Key>> = RefCell::new(HashSet::new());
    static FAILED: RefCell<HashSet<Key>> = RefCell::new(HashSet::new());
    static TICKING: Cell<bool> = const { Cell::new(false) };
    static EPOCH: Instant = Instant::now();
}

/// Shows the clip at `path` looping in `picture`, fitted inside `width` × `height`.
pub fn play(picture: &gtk::Picture, path: &str, width: i32, height: i32) {
    picture.set_size_request(width, height);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_can_shrink(true);
    let scale = display_scale();
    let key = Key {
        path: path.to_string(),
        width: (width as f64 * scale).ceil() as u32,
        height: (height as f64 * scale).ceil() as u32,
        still: crate::prefs::get().reduce_motion,
    };
    let clip = lookup(&key);
    let mut shown = usize::MAX;
    if let Some(clip) = &clip {
        shown = current_frame(clip);
        picture.set_paintable(Some(&clip.frames[shown]));
    }
    PLAYERS.with(|players| {
        let mut players = players.borrow_mut();
        players.retain(|p| p.picture.upgrade().is_some_and(|p| &p != picture));
        players.push(Player { picture: picture.downgrade(), key, clip, shown, hidden_since: None });
    });
    if !TICKING.replace(true) {
        glib::timeout_add_local(TICK, tick);
    }
}

fn display_scale() -> f64 {
    gdk::Display::default()
        .map(|d| {
            let monitors = d.monitors();
            (0..monitors.n_items())
                .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
                .map(|m| m.scale())
                .fold(1.0, f64::max)
        })
        .unwrap_or(1.0)
}

fn current_frame(clip: &Clip) -> usize {
    let elapsed = EPOCH.with(Instant::elapsed);
    (elapsed.as_nanos() / clip.frame_time.as_nanos().max(1)) as usize % clip.frames.len()
}

fn tick() -> glib::ControlFlow {
    let now = Instant::now();
    let mut wanted = Vec::new();
    let alive = PLAYERS.with(|players| {
        let mut players = players.borrow_mut();
        players.retain_mut(|p| {
            let Some(picture) = p.picture.upgrade() else {
                if let Some(clip) = p.clip.take() {
                    keep_recent(clip);
                }
                return false;
            };
            if !on_screen(&picture) {
                let since = *p.hidden_since.get_or_insert(now);
                if now.duration_since(since) > RELEASE_AFTER
                    && let Some(clip) = p.clip.take()
                {
                    keep_recent(clip);
                }
                return true;
            }
            p.hidden_since = None;
            if p.clip.is_none() {
                p.clip = lookup(&p.key);
                p.shown = usize::MAX;
                if p.clip.is_none() {
                    wanted.push(p.key.clone());
                }
            }
            if let Some(clip) = &p.clip {
                let frame = current_frame(clip);
                if frame != p.shown {
                    picture.set_paintable(Some(&clip.frames[frame]));
                    p.shown = frame;
                }
            }
            true
        });
        !players.is_empty()
    });
    for key in wanted {
        request(key);
    }
    if !alive {
        TICKING.set(false);
        return glib::ControlFlow::Break;
    }
    glib::ControlFlow::Continue
}

fn on_screen(picture: &gtk::Picture) -> bool {
    if !picture.is_mapped() {
        return false;
    }
    let Some(view) = picture.ancestor(gtk::ScrolledWindow::static_type()) else { return true };
    let Some(b) = picture.compute_bounds(&view) else { return false };
    b.y() + b.height() > 0.0 && b.y() < view.height() as f32 && b.x() + b.width() > 0.0 && b.x() < view.width() as f32
}

fn lookup(key: &Key) -> Option<Rc<Clip>> {
    let clip = CLIPS.with(|c| c.borrow().get(key).and_then(Weak::upgrade))?;
    keep_recent(clip.clone());
    Some(clip)
}

fn keep_recent(clip: Rc<Clip>) {
    RECENT.with(|recent| {
        let mut recent = recent.borrow_mut();
        recent.retain(|c| !Rc::ptr_eq(c, &clip));
        recent.push_front(clip);
        let mut total = 0;
        recent.retain(|c| {
            total += c.bytes;
            total <= RECENT_BUDGET
        });
    });
}

type Job = (Key, async_channel::Sender<(Key, Result<Decoded, String>)>);

fn request(key: Key) {
    if FAILED.with(|f| f.borrow().contains(&key)) || !PENDING.with(|p| p.borrow_mut().insert(key.clone())) {
        return;
    }
    static QUEUE: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    thread_local! {
        static RESULTS: async_channel::Sender<(Key, Result<Decoded, String>)> = {
            let (tx, rx) = async_channel::unbounded();
            glib::spawn_future_local(async move {
                while let Ok((key, result)) = rx.recv().await {
                    finish(key, result);
                }
            });
            tx
        };
    }
    let queue = QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..DECODERS {
            let rx = rx.clone();
            let _ = std::thread::Builder::new().name(format!("pulse-clip-{i}")).spawn(move || {
                loop {
                    let job = rx.lock().ok().and_then(|rx| rx.recv().ok());
                    let Some((key, reply)) = job else { break };
                    let result = decode(&key);
                    if reply.send_blocking((key, result)).is_err() {
                        break;
                    }
                }
            });
        }
        Mutex::new(tx)
    });
    let reply = RESULTS.with(Clone::clone);
    if let Ok(queue) = queue.lock() {
        let _ = queue.send((key, reply));
    }
}

fn finish(key: Key, result: Result<Decoded, String>) {
    PENDING.with(|p| p.borrow_mut().remove(&key));
    let decoded = match result {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("Could not decode {}: {e}", key.path);
            FAILED.with(|f| f.borrow_mut().insert(key));
            return;
        }
    };
    let stride = decoded.width * 4;
    let bytes = decoded.frames.iter().map(Vec::len).sum();
    let frames = decoded
        .frames
        .into_iter()
        .map(|data| {
            gdk::MemoryTexture::new(
                decoded.width as i32,
                decoded.height as i32,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_owned(data),
                stride,
            )
            .upcast()
        })
        .collect();
    let clip = Rc::new(Clip { frames, frame_time: decoded.frame_time, bytes });
    CLIPS.with(|c| {
        let mut clips = c.borrow_mut();
        clips.retain(|_, w| w.strong_count() > 0);
        clips.insert(key, Rc::downgrade(&clip));
    });
    keep_recent(clip);
}

fn decode(key: &Key) -> Result<Decoded, String> {
    // ffmpeg's built-in VP9 decoder drops the alpha plane Telegram stickers use.
    if key.path.ends_with(".webm")
        && let Ok(d) = run_ffmpeg(key, Some("libvpx-vp9"))
    {
        return Ok(d);
    }
    run_ffmpeg(key, None)
}

fn run_ffmpeg(key: &Key, codec: Option<&str>) -> Result<Decoded, String> {
    let mut command = Command::new("ffmpeg");
    command.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-threads", "1"]);
    if let Some(codec) = codec {
        command.args(["-c:v", codec]);
    }
    let filter = format!(
        "fps={FPS},scale=w='min({w}\\,iw)':h='min({h}\\,ih)':force_original_aspect_ratio=decrease",
        w = key.width.max(1),
        h = key.height.max(1)
    );
    command.arg("-i").arg(&key.path).args(["-an", "-t", &MAX_SECONDS.to_string(), "-vf", &filter]);
    if key.still {
        command.args(["-frames:v", "1"]);
    }
    command.args(["-pix_fmt", "rgba", "-c:v", "pam", "-f", "image2pipe", "-"]);
    let mut child =
        command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let mut out = BufReader::new(child.stdout.take().ok_or("ffmpeg has no output")?);
    let result = read_frames(&mut out);
    drop(out);
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn read_frames(out: &mut impl BufRead) -> Result<Decoded, String> {
    let (mut width, mut height) = (0, 0);
    let mut frames = Vec::new();
    let mut bytes = 0;
    let mut step = 1;
    let mut index = 0usize;
    while let Some((w, h)) = read_pam_header(out)? {
        let mut frame = vec![0u8; w * h * 4];
        out.read_exact(&mut frame).map_err(|e| e.to_string())?;
        (width, height) = (w, h);
        index += 1;
        if !(index - 1).is_multiple_of(step) {
            continue;
        }
        bytes += frame.len();
        frames.push(frame);
        if bytes > CLIP_BUDGET {
            if step == 2 {
                frames.pop();
                break;
            }
            step = 2;
            let mut i = 0;
            frames.retain(|_| {
                i += 1;
                i % 2 == 1
            });
            bytes = frames.iter().map(Vec::len).sum();
        }
    }
    if frames.is_empty() {
        return Err("no frames".into());
    }
    Ok(Decoded { width, height, frames, frame_time: Duration::from_secs(1) * step as u32 / FPS })
}

fn read_pam_header(out: &mut impl BufRead) -> Result<Option<(usize, usize)>, String> {
    let (mut width, mut height, mut depth) = (0, 0, 0);
    let mut line = String::new();
    loop {
        line.clear();
        if out.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Ok(None);
        }
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next().and_then(|v| v.parse::<usize>().ok())) {
            (Some("WIDTH"), Some(v)) => width = v,
            (Some("HEIGHT"), Some(v)) => height = v,
            (Some("DEPTH"), Some(v)) => depth = v,
            (Some("ENDHDR"), _) => break,
            _ => {}
        }
    }
    if width == 0 || height == 0 || depth != 4 {
        return Err(format!("unexpected frame {width}×{height}×{depth}"));
    }
    Ok(Some((width, height)))
}

#[cfg(test)]
#[test]
fn decodes_sticker_frames() {
    let dir = std::env::temp_dir().join(format!("pulse-clip-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sticker.webm");
    let made = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("color=c=red:s=512x512:r=30:d=1,format=yuva420p,geq=lum=81:cb=90:cr=240:a='255*between(X\\,128\\,383)*between(Y\\,128\\,383)'")
        .args(["-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-auto-alt-ref", "0"])
        .arg(&path)
        .status()
        .unwrap();
    assert!(made.success());
    let key = Key { path: path.to_str().unwrap().into(), width: 160, height: 120, still: false };
    let clip = decode(&key).unwrap();
    assert_eq!((clip.width, clip.height), (120, 120), "Fits inside the box, keeping the aspect ratio");
    assert_eq!(clip.frames.len(), 30);
    assert_eq!(clip.frame_time, Duration::from_secs(1) / 30);
    let pixel = |x: usize, y: usize| clip.frames[0][(y * clip.width + x) * 4 + 3];
    assert_eq!(pixel(2, 2), 0, "Sticker transparency survives decoding");
    assert!(pixel(60, 60) > 245);
    let still = decode(&Key { still: true, ..key.clone() }).unwrap();
    assert_eq!(still.frames.len(), 1);
    let small = decode(&Key { width: 4000, height: 4000, ..key }).unwrap();
    assert_eq!((small.width, small.height), (512, 512), "Never decodes larger than the source");
    assert!(small.frames.iter().map(Vec::len).sum::<usize>() <= CLIP_BUDGET);
    std::fs::remove_dir_all(dir).unwrap();
}
