use crate::source::Sample;
use crossbeam_channel::{bounded, RecvTimeoutError, Sender};
use std::{
    fs::File,
    io::{BufWriter, Write},
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub enum Fmt {
    Csv,
    Txt,
    Log,
}

impl Fmt {
    pub fn ext(self) -> &'static str {
        match self {
            Fmt::Csv => "csv",
            Fmt::Txt => "txt",
            Fmt::Log => "log",
        }
    }
}

pub fn start(
    path: &str,
    fmt: Fmt,
) -> std::io::Result<(Sender<Arc<Sample>>, JoinHandle<()>)> {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    if let Fmt::Csv = fmt {
        writeln!(w, "time_s,channel,value")?;
    }
    let (tx, rx) = bounded::<Arc<Sample>>(1_000_000);
    let jh = thread::spawn(move || {
        let mut last_flush = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(s) => {
                    match fmt {
                        Fmt::Csv => {
                            for (k, v) in &s.vals {
                                let _ = writeln!(w, "{:.6},{},{}", s.t, k, v);
                            }
                        }
                        Fmt::Txt | Fmt::Log => {
                            let _ = writeln!(w, "{:.3}\t{}", s.t, s.raw);
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if last_flush.elapsed() > Duration::from_secs(1) {
                let _ = w.flush();
                last_flush = Instant::now();
            }
        }
        let _ = w.flush();
    });
    Ok((tx, jh))
}