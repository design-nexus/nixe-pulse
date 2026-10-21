//! Keep the portal session and PipeWire remote alive for the complete capture.
use crate::engine::Engine;
use ashpd::desktop::{
    CreateSessionOptions,
    screencast::{OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType, StartCastOptions},
};
use std::{
    io::Read,
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
pub async fn run(engine: Arc<Mutex<Engine>>, user: i64, stop: Arc<AtomicBool>) -> Result<(), String> {
    let portal = Screencast::new().await.map_err(|e| e.to_string())?;
    let session = portal.create_session(CreateSessionOptions::default()).await.map_err(|e| e.to_string())?;
    portal
        .select_sources(&session, SelectSourcesOptions::default().set_sources(SourceType::Monitor | SourceType::Window))
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;
    let response = portal
        .start(&session, None, StartCastOptions::default())
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;
    let stream = response.streams().first().ok_or("No screen was selected")?;
    let remote = portal.open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default()).await.map_err(|e| e.to_string())?;
    let fd = remote.as_raw_fd();
    let mut cmd = Command::new("gst-launch-1.0");
    cmd.args([
        "-q",
        "pipewiresrc",
        &format!("fd={fd}"),
        &format!("path={}", stream.pipe_wire_node_id()),
        "!",
        "videoconvert",
        "!",
        "videoscale",
        "!",
        "videorate",
        "!",
        "video/x-raw,format=I420,width=640,height=360,framerate=15/1",
        "!",
        "fdsink",
        "fd=1",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    // Only the child clears CLOEXEC; the parent's descriptor remains protected.
    unsafe {
        cmd.pre_exec(move || {
            if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().ok_or("Missing capture stream")?;
    let end = stop.clone();
    let reader = std::thread::spawn(move || {
        let mut frame = vec![0; 640 * 360 * 3 / 2];
        while !end.load(Ordering::Relaxed) {
            if stdout.read_exact(&mut frame).is_err() {
                break;
            }
            if engine.lock().unwrap().screen(user, &frame, 640, 360).is_err() {
                break;
            }
        }
    });
    let result = loop {
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            break Ok(());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break Err(format!("Screen capture ended ({status})"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    let _ = child.wait();
    let _ = reader.join();
    let _ = session.close().await;
    result
}
