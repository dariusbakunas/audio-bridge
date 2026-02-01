use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, TryRecvError};

use crate::state::PlayerStatus;

#[derive(Debug, Clone)]
pub enum BridgeCommand {
    Play { path: PathBuf, ext_hint: String },
    PauseToggle,
    Quit,
    ListDevices {
        resp_tx: crossbeam_channel::Sender<Result<Vec<String>>>,
    },
    SetDevice {
        name: String,
        resp_tx: crossbeam_channel::Sender<Result<()>>,
    },
}

#[derive(Clone)]
pub struct BridgePlayer {
    pub(crate) cmd_tx: Sender<BridgeCommand>,
}

pub fn spawn_bridge_worker(
    addr: SocketAddr,
    cmd_rx: Receiver<BridgeCommand>,
    cmd_tx: Sender<BridgeCommand>,
    status: Arc<Mutex<PlayerStatus>>,
    queue: Arc<Mutex<crate::state::QueueState>>,
) {
    std::thread::spawn(move || {
        let device_waiter: Arc<Mutex<Option<crossbeam_channel::Sender<DeviceEvent>>>> =
            Arc::new(Mutex::new(None));
        let mut stream = connect_loop(
            addr,
            status.clone(),
            queue.clone(),
            cmd_tx.clone(),
            device_waiter.clone(),
        );
        let mut paused = false;

        loop {
            let cmd = match cmd_rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            };

            match cmd {
                BridgeCommand::Quit => {
                    let _ = audio_bridge_proto::write_frame(
                        &mut stream,
                        audio_bridge_proto::FrameKind::Next,
                        &[],
                    );
                    break;
                }
                BridgeCommand::PauseToggle => {
                    paused = !paused;
                    let kind = if paused {
                        audio_bridge_proto::FrameKind::Pause
                    } else {
                        audio_bridge_proto::FrameKind::Resume
                    };
                    if audio_bridge_proto::write_frame(&mut stream, kind, &[]).is_ok() {
                        if let Ok(mut s) = status.lock() {
                            s.paused = paused;
                        }
                    }
                }
                BridgeCommand::Play { path, ext_hint } => {
                    let mut next = Some((path, ext_hint));
                    while let Some((path, ext_hint)) = next.take() {
                        tracing::info!(path = %path.display(), "bridge play");
                        if let Ok(mut s) = status.lock() {
                            s.now_playing = Some(path.clone());
                            s.paused = false;
                            s.elapsed_ms = Some(0);
                            s.auto_advance_in_flight = false;
                        }
                        match send_one_track_over_existing_connection(
                            &mut stream,
                            &cmd_rx,
                            &mut paused,
                            path,
                            ext_hint,
                            &device_waiter,
                        ) {
                            Ok(Flow::Continue) => break,
                            Ok(Flow::SwitchTo { path, ext_hint }) => {
                                next = Some((path, ext_hint));
                            }
                            Ok(Flow::Quit) => return,
                            Err(e) => {
                                if is_network_error(&e) {
                                    tracing::warn!("bridge connection lost; reconnecting");
                                    stream = connect_loop(
                                        addr,
                                        status.clone(),
                                        queue.clone(),
                                        cmd_tx.clone(),
                                        device_waiter.clone(),
                                    );
                                    continue;
                                }
                                tracing::error!("playback failed: {e:#}");
                                break;
                            }
                        }
                    }
                }
                BridgeCommand::ListDevices { resp_tx } => {
                    let _ = handle_list_devices(
                        &mut stream,
                        &device_waiter,
                        resp_tx,
                    );
                }
                BridgeCommand::SetDevice { name, resp_tx } => {
                    let _ = handle_set_device(
                        &mut stream,
                        &device_waiter,
                        name,
                        resp_tx,
                    );
                }
            }
        }
    });
}

#[derive(Debug)]
enum Flow {
    Continue,
    SwitchTo { path: PathBuf, ext_hint: String },
    Quit,
}

fn connect_loop(
    addr: SocketAddr,
    status: Arc<Mutex<PlayerStatus>>,
    queue: Arc<Mutex<crate::state::QueueState>>,
    cmd_tx: Sender<BridgeCommand>,
    device_waiter: Arc<Mutex<Option<crossbeam_channel::Sender<DeviceEvent>>>>,
) -> TcpStream {
    let mut delay = Duration::from_millis(250);
    loop {
        match connect_and_handshake(
            addr,
            status.clone(),
            queue.clone(),
            cmd_tx.clone(),
            device_waiter.clone(),
        ) {
            Ok(stream) => {
                tracing::info!(addr = %addr, "bridge connected");
                return stream;
            }
            Err(_) => {
                tracing::warn!(addr = %addr, delay_ms = delay.as_millis(), "bridge connect failed");
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_secs(5));
            }
        }
    }
}

fn connect_and_handshake(
    addr: SocketAddr,
    status: Arc<Mutex<PlayerStatus>>,
    queue: Arc<Mutex<crate::state::QueueState>>,
    cmd_tx: Sender<BridgeCommand>,
    device_waiter: Arc<Mutex<Option<crossbeam_channel::Sender<DeviceEvent>>>>,
) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(addr).with_context(|| format!("connect {addr}"))?;
    stream.set_nodelay(true).ok();
    stream.set_write_timeout(Some(Duration::from_millis(200))).ok();

    audio_bridge_proto::write_prelude(&mut stream).context("write prelude")?;
    let mut stream_rx = stream.try_clone().context("try_clone stream for rx")?;
    audio_bridge_proto::read_prelude(&mut stream_rx).context("read prelude")?;
    spawn_bridge_reader(stream_rx, status, queue, cmd_tx, device_waiter);
    Ok(stream)
}

fn spawn_bridge_reader(
    mut stream_rx: TcpStream,
    status: Arc<Mutex<PlayerStatus>>,
    queue: Arc<Mutex<crate::state::QueueState>>,
    cmd_tx: Sender<BridgeCommand>,
    device_waiter: Arc<Mutex<Option<crossbeam_channel::Sender<DeviceEvent>>>>,
) {
    std::thread::spawn(move || loop {
        let (kind, len) = match audio_bridge_proto::read_frame_header(&mut stream_rx) {
            Ok(x) => x,
            Err(_) => {
                tracing::warn!("bridge reader disconnected");
                break;
            }
        };
        let mut payload = vec![0u8; len as usize];
        if stream_rx.read_exact(&mut payload).is_err() {
            break;
        }
        match kind {
            audio_bridge_proto::FrameKind::TrackInfo => {
                if let Ok((sr, _ch, dur)) = audio_bridge_proto::decode_track_info(&payload) {
                    if let Ok(mut s) = status.lock() {
                        s.sample_rate = Some(sr);
                        if dur.is_some() {
                            s.duration_ms = dur;
                        }
                    }
                }
            }
            audio_bridge_proto::FrameKind::PlaybackPos => {
                if let Ok((frames, paused)) = audio_bridge_proto::decode_playback_pos(&payload) {
                    if let Ok(mut s) = status.lock() {
                        s.paused = paused;
                        if let Some(sr) = s.sample_rate {
                            if sr > 0 {
                                let elapsed = frames.saturating_mul(1000) / sr as u64;
                                s.elapsed_ms = Some(elapsed);
                            }
                        }
                        if let (Some(elapsed), Some(duration)) = (s.elapsed_ms, s.duration_ms) {
                            if elapsed > duration {
                                s.elapsed_ms = Some(duration);
                            }
                        }
                        if !s.auto_advance_in_flight {
                            if let (Some(elapsed), Some(duration)) = (s.elapsed_ms, s.duration_ms) {
                                if elapsed + 50 >= duration && !s.user_paused {
                                    drop(s);
                                    if let Some(path) = pop_next_from_queue(&queue) {
                                        let ext_hint = path
                                            .extension()
                                            .and_then(|ext| ext.to_str())
                                            .unwrap_or("")
                                            .to_ascii_lowercase();
                                        let _ = cmd_tx.send(BridgeCommand::Play { path, ext_hint });
                                        if let Ok(mut s2) = status.lock() {
                                            s2.auto_advance_in_flight = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            audio_bridge_proto::FrameKind::DeviceList => {
                if let Ok(list) = audio_bridge_proto::decode_device_list(&payload) {
                    if let Some(tx) = device_waiter.lock().ok().and_then(|mut g| g.take()) {
                        let _ = tx.send(DeviceEvent::List(list));
                    }
                }
            }
            audio_bridge_proto::FrameKind::DeviceSet => {
                if let Some(tx) = device_waiter.lock().ok().and_then(|mut g| g.take()) {
                    let _ = tx.send(DeviceEvent::SetOk);
                }
            }
            audio_bridge_proto::FrameKind::Error => {
                let msg = String::from_utf8_lossy(&payload).to_string();
                if let Some(tx) = device_waiter.lock().ok().and_then(|mut g| g.take()) {
                    let _ = tx.send(DeviceEvent::Error(msg));
                }
            }
            _ => {}
        }
    });
}

#[derive(Debug)]
enum DeviceEvent {
    List(Vec<String>),
    SetOk,
    Error(String),
}

fn handle_list_devices(
    stream: &mut TcpStream,
    device_waiter: &Arc<Mutex<Option<crossbeam_channel::Sender<DeviceEvent>>>>,
    resp_tx: crossbeam_channel::Sender<Result<Vec<String>>>,
) -> Result<()> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    {
        if let Ok(mut g) = device_waiter.lock() {
            *g = Some(tx);
        }
    }
    audio_bridge_proto::write_frame(stream, audio_bridge_proto::FrameKind::ListDevices, &[])?;
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(DeviceEvent::List(list)) => {
            let _ = resp_tx.send(Ok(list));
        }
        Ok(DeviceEvent::Error(msg)) => {
            let _ = resp_tx.send(Err(anyhow::anyhow!(msg)));
        }
        _ => {
            let _ = resp_tx.send(Err(anyhow::anyhow!("device list timeout")));
        }
    }
    Ok(())
}

fn handle_set_device(
    stream: &mut TcpStream,
    device_waiter: &Arc<Mutex<Option<crossbeam_channel::Sender<DeviceEvent>>>>,
    name: String,
    resp_tx: crossbeam_channel::Sender<Result<()>>,
) -> Result<()> {
    let payload = audio_bridge_proto::encode_device_selector(&name)?;
    let (tx, rx) = crossbeam_channel::bounded(1);
    {
        if let Ok(mut g) = device_waiter.lock() {
            *g = Some(tx);
        }
    }
    audio_bridge_proto::write_frame(stream, audio_bridge_proto::FrameKind::SetDevice, &payload)?;
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(DeviceEvent::SetOk) => {
            let _ = resp_tx.send(Ok(()));
        }
        Ok(DeviceEvent::Error(msg)) => {
            let _ = resp_tx.send(Err(anyhow::anyhow!(msg)));
        }
        _ => {
            let _ = resp_tx.send(Err(anyhow::anyhow!("device set timeout")));
        }
    }
    Ok(())
}

fn pop_next_from_queue(queue: &Arc<Mutex<crate::state::QueueState>>) -> Option<PathBuf> {
    let mut q = queue.lock().ok()?;
    if q.items.is_empty() {
        None
    } else {
        Some(q.items.remove(0))
    }
}

fn write_all_interruptible(
    stream: &mut TcpStream,
    buf: &[u8],
    cmd_rx: &Receiver<BridgeCommand>,
    paused: &mut bool,
    device_waiter: &Arc<Mutex<Option<crossbeam_channel::Sender<DeviceEvent>>>>,
) -> Result<Flow> {
    let mut off = 0usize;

    while off < buf.len() {
        match cmd_rx.try_recv() {
            Ok(BridgeCommand::Quit) => return Ok(Flow::Quit),
            Ok(BridgeCommand::Play { path, ext_hint }) => return Ok(Flow::SwitchTo { path, ext_hint }),
            Ok(BridgeCommand::PauseToggle) => {
                *paused = !*paused;
                let kind = if *paused {
                    audio_bridge_proto::FrameKind::Pause
                } else {
                    audio_bridge_proto::FrameKind::Resume
                };
                audio_bridge_proto::write_frame(&mut *stream, kind, &[])
                    .with_context(|| format!("write {kind:?}"))?;
            }
            Ok(BridgeCommand::ListDevices { resp_tx }) => {
                let _ = handle_list_devices(stream, device_waiter, resp_tx);
            }
            Ok(BridgeCommand::SetDevice { name, resp_tx }) => {
                let _ = handle_set_device(stream, device_waiter, name, resp_tx);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return Ok(Flow::Quit),
        }

        match stream.write(&buf[off..]) {
            Ok(0) => return Err(anyhow::anyhow!("socket closed while writing")).map_err(Into::into),
            Ok(n) => off += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(e) => return Err(e).context("write to socket").map_err(Into::into),
        }
    }

    Ok(Flow::Continue)
}

fn is_network_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        if let Some(ioe) = cause.downcast_ref::<io::Error>() {
            matches!(
                ioe.kind(),
                io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::TimedOut
                    | io::ErrorKind::WouldBlock
            )
        } else {
            false
        }
    })
}

fn write_frame_interruptible(
    stream: &mut TcpStream,
    kind: audio_bridge_proto::FrameKind,
    payload: &[u8],
    cmd_rx: &Receiver<BridgeCommand>,
    paused: &mut bool,
    device_waiter: &Arc<Mutex<Option<crossbeam_channel::Sender<DeviceEvent>>>>,
) -> Result<Flow> {
    let frame = audio_bridge_proto::encode_frame(kind, payload)
        .context("encode frame")?;
    write_all_interruptible(stream, &frame, cmd_rx, paused, device_waiter)
}

fn send_one_track_over_existing_connection(
    stream: &mut TcpStream,
    cmd_rx: &Receiver<BridgeCommand>,
    paused: &mut bool,
    path: PathBuf,
    ext_hint: String,
    device_waiter: &Arc<Mutex<Option<crossbeam_channel::Sender<DeviceEvent>>>>,
) -> Result<Flow> {
    let _ = audio_bridge_proto::write_frame(&mut *stream, audio_bridge_proto::FrameKind::Next, &[]);

    let begin = audio_bridge_proto::encode_begin_file_payload(&ext_hint)
        .context("encode BEGIN_FILE")?;
    match write_frame_interruptible(
        stream,
        audio_bridge_proto::FrameKind::BeginFile,
        &begin,
        cmd_rx,
        paused,
        device_waiter,
    )? {
        Flow::Continue => {}
        other => return Ok(other),
    }

    if *paused {
        audio_bridge_proto::write_frame(&mut *stream, audio_bridge_proto::FrameKind::Pause, &[])
            .context("write PAUSE")?;
    }

    let mut file = File::open(&path).with_context(|| format!("open {:?}", path))?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).context("read file")?;
        if n == 0 {
            break;
        }
        match write_frame_interruptible(
            stream,
            audio_bridge_proto::FrameKind::FileChunk,
            &buf[..n],
            cmd_rx,
            paused,
            device_waiter,
        )? {
            Flow::Continue => {}
            other => return Ok(other),
        }
    }

    match write_frame_interruptible(
        stream,
        audio_bridge_proto::FrameKind::EndFile,
        &[],
        cmd_rx,
        paused,
        device_waiter,
    )? {
        Flow::Continue => {}
        other => return Ok(other),
    }

    Ok(Flow::Continue)
}
