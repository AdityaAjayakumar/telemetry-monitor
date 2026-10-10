use crate::parser::parse_line;
use crate::preset::Preset;
use crossbeam_channel::Sender;
use std::{
    io::{BufRead, BufReader, ErrorKind, Read},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    thread,
    time::{Duration, Instant},
};

pub struct Sample {
    pub t: f64,
    pub raw: String,
    pub vals: Vec<(String, f64)>,
}

pub enum Source {
    Serial { port: String, baud: u32 },
    TcpListen(String),
    TcpConnect(String),
}

pub type StatusFn = Arc<dyn Fn(String) + Send + Sync>;
pub type LogSlot = Arc<Mutex<Option<Sender<Arc<Sample>>>>>;
pub type SharedPreset = Arc<RwLock<Preset>>;

pub fn spawn(
    src: Source,
    ui_tx: Sender<Arc<Sample>>,
    log: LogSlot,
    preset: SharedPreset,
    t0: Instant,
    stop: Arc<AtomicBool>,
    status: StatusFn,
) {
    thread::spawn(move || {
        let tmo = Duration::from_millis(100);
        match src {
            Source::Serial { port, baud } => {
                match serialport::new(&port, baud).timeout(tmo).open() {
                    Ok(p) => {
                        status(format!("Connected {port} @ {baud}"));
                        pump(p, &ui_tx, &log, &preset, t0, &stop);
                        status("Disconnected".into());
                    }
                    Err(e) => status(format!("Open failed: {e}")),
                }
            }
            Source::TcpConnect(addr) => match TcpStream::connect(&addr) {
                Ok(s) => {
                    s.set_read_timeout(Some(tmo)).ok();
                    status(format!("Connected to {addr}"));
                    pump(s, &ui_tx, &log, &preset, t0, &stop);
                    status("Disconnected".into());
                }
                Err(e) => status(format!("Connect failed: {e}")),
            },
            Source::TcpListen(addr) => match TcpListener::bind(&addr) {
                Ok(l) => {
                    l.set_nonblocking(true).ok();
                    status(format!("Listening on {addr}"));
                    while !stop.load(Ordering::Relaxed) {
                        match l.accept() {
                            Ok((s, peer)) => {
                                s.set_nonblocking(false).ok();
                                s.set_read_timeout(Some(tmo)).ok();
                                status(format!("Client {peer}"));
                                pump(s, &ui_tx, &log, &preset, t0, &stop);
                                status("Client left, listening".into());
                            }
                            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(50));
                            }
                            Err(e) => {
                                status(format!("Accept error: {e}"));
                                break;
                            }
                        }
                    }
                }
                Err(e) => status(format!("Bind failed: {e}")),
            },
        }
    });
}

fn pump<R: Read>(
    r: R,
    ui_tx: &Sender<Arc<Sample>>,
    log: &LogSlot,
    preset: &SharedPreset,
    t0: Instant,
    stop: &AtomicBool,
) {
    let mut rd = BufReader::with_capacity(1 << 16, r);
    let mut buf: Vec<u8> = Vec::with_capacity(256);
    let mut first = true; // drop the first (usually partial) line
    while !stop.load(Ordering::Relaxed) {
        match rd.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                if buf.ends_with(b"\n") {
                    if first {
                        first = false;
                        buf.clear();
                        continue;
                    }
                    let text = String::from_utf8_lossy(&buf);
                    let line = text.trim_end_matches(['\r', '\n']);
                    let s = Arc::new(Sample {
                        t: t0.elapsed().as_secs_f64(),
                        raw: line.to_string(),
                        vals: parse_line(line, &preset.read().unwrap()),
                    });
                    if let Some(tx) = log.lock().unwrap().as_ref() {
                        let _ = tx.send(s.clone());
                    }
                    let _ = ui_tx.try_send(s); // drop if UI is behind
                    buf.clear();
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted
                ) =>
            {
                continue
            }
            Err(_) => break,
        }
    }
}