use crate::store::{Series, Store};
use crate::{Bar, Trace};
use slint::Color;
use std::{
    collections::{HashSet, VecDeque},
    fmt::Write,
};

#[derive(Default)]
pub struct Frame {
    pub traces: Vec<Trace>,
    pub bars: Vec<Bar>,
    pub y_top: String,
    pub y_bot: String,
}

fn nat_key(name: &str) -> (String, u32) {
    let digits = name.chars().rev().take_while(|c| c.is_ascii_digit()).count();
    let (a, b) = name.split_at(name.len() - digits);
    (a.to_string(), b.parse().unwrap_or(0))
}

fn color(i: usize) -> Color {
    let h = (i as f32 * 137.508) % 360.0;
    let (s, v) = (0.65f32, 0.95f32);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::from_rgb_u8(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// min/max per pixel column, keeping time order inside the column
fn decimate(
    pts: &VecDeque<(f64, f64)>,
    t0: f64,
    t1: f64,
    cols: usize,
) -> Vec<(f32, f64)> {
    fn flush(out: &mut Vec<(f32, f64)>, col: usize, mn: (usize, f64), mx: (usize, f64)) {
        if col == usize::MAX {
            return;
        }
        let x = col as f32 + 0.5;
        if mn.0 == mx.0 {
            out.push((x, mn.1));
        } else if mn.0 < mx.0 {
            out.push((x, mn.1));
            out.push((x, mx.1));
        } else {
            out.push((x, mx.1));
            out.push((x, mn.1));
        }
    }
    let start = pts.partition_point(|p| p.0 < t0);
    let span = (t1 - t0).max(1e-9);
    let mut out = Vec::with_capacity(cols * 2);
    let (mut col, mut idx) = (usize::MAX, 0usize);
    let (mut mn, mut mx) = ((0usize, 0.0f64), (0usize, 0.0f64));
    for &(t, v) in pts.range(start..) {
        let c = ((((t - t0) / span) * cols as f64) as usize).min(cols - 1);
        if c != col {
            flush(&mut out, col, mn, mx);
            col = c;
            mn = (idx, v);
            mx = (idx, v);
        } else {
            if v < mn.1 {
                mn = (idx, v);
            }
            if v > mx.1 {
                mx = (idx, v);
            }
        }
        idx += 1;
    }
    flush(&mut out, col, mn, mx);
    out
}

fn ordered(store: &Store) -> Vec<(usize, &Series)> {
    let mut v: Vec<_> = store.series.iter().enumerate().collect();
    if !store.keep_order {
        v.sort_by_key(|(_, s)| nat_key(&s.name));
    }
    v
}

fn range_or_default(lo: f64, hi: f64) -> (f64, f64) {
    let (mut lo, mut hi) = if lo > hi { (0.0, 1.0) } else { (lo, hi) };
    if (hi - lo).abs() < 1e-9 {
        hi = lo + 1.0;
    }
    let pad = (hi - lo) * 0.05;
    lo -= pad;
    hi += pad;
    (lo, hi)
}

fn legend(list: &[(usize, &Series)], hidden: &HashSet<String>) -> Vec<Trace> {
    list.iter()
                .map(|(i, s)| Trace {
            name: s.name.clone().into(),
            color: color(*i),
            visible: !hidden.contains(&s.name),
            commands: Default::default(),
            value: s
                .pts
                .back()
                .map(|p| format!("{:.2}", p.1))
                .unwrap_or_default()
                .into(),
        })
        .collect()
}

pub fn lines(
    store: &Store,
    hidden: &HashSet<String>,
    window: f64,
    w: f32,
    h: f32,
) -> Frame {
    let t1 = store.t_max();
    let t0 = t1 - window;
    let cols = (w.max(50.0)) as usize;
    let list = ordered(store);
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    let mut data = Vec::new();
    for (_, s) in &list {
        let d = if hidden.contains(&s.name) {
            vec![]
        } else {
            decimate(&s.pts, t0, t1, cols)
        };
        for &(_, v) in &d {
            lo = lo.min(v);
            hi = hi.max(v);
        }
        data.push(d);
    }
    let (lo, hi) = range_or_default(lo, hi);
    let ymap = |v: f64| ((1.0 - (v - lo) / (hi - lo)) as f32) * h;
    let mut traces = legend(&list, hidden);
    for (tr, d) in traces.iter_mut().zip(&data) {
        let mut cmd = String::with_capacity(d.len() * 12);
        for (k, (x, v)) in d.iter().enumerate() {
            let _ = write!(
                cmd,
                "{}{:.1} {:.1} ",
                if k == 0 { "M" } else { "L" },
                x,
                ymap(*v)
            );
        }
        tr.commands = cmd.into();
    }
    Frame {
        traces,
        bars: vec![],
        y_top: format!("{hi:.2}"),
        y_bot: format!("{lo:.2}"),
    }
}

pub fn boxes(
    store: &Store,
    hidden: &HashSet<String>,
    window: f64,
    w: f32,
    h: f32,
) -> Frame {
    let t1 = store.t_max();
    let t0 = t1 - window;
    let list = ordered(store);
    let mut f = Frame {
        traces: legend(&list, hidden),
        ..Default::default()
    };
    let mut stats = Vec::new(); // (idx, name, min, q1, med, q3, max)
    for (i, s) in list.iter().filter(|(_, s)| !hidden.contains(&s.name)) {
        let start = s.pts.partition_point(|p| p.0 < t0);
        let len = s.pts.len() - start;
        if len == 0 {
            continue;
        }
        let step = (len / 4000).max(1);
        let mut v: Vec<f64> = s.pts.range(start..).step_by(step).map(|p| p.1).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let q = |p: f64| v[((v.len() - 1) as f64 * p) as usize];
        stats.push((*i, s.name.clone(), q(0.0), q(0.25), q(0.5), q(0.75), q(1.0)));
    }
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for s in &stats {
        lo = lo.min(s.2);
        hi = hi.max(s.6);
    }
    let (lo, hi) = range_or_default(lo, hi);
    let ymap = |v: f64| ((1.0 - (v - lo) / (hi - lo)) as f32) * h;
    let slot = w / stats.len().max(1) as f32;
    for (k, (i, name, mn, q1, md, q3, mx)) in stats.iter().enumerate() {
        f.bars.push(Bar {
            x: (k as f32 + 0.5) * slot,
            w: (slot * 0.6).clamp(4.0, 60.0),
            y_lo: ymap(*mn),
            y_hi: ymap(*mx),
            y_a: ymap(*q3),
            y_b: ymap(*q1),
            y_mid: ymap(*md),
            color: color(*i),
            label: name.clone().into(),
        });
    }
    f.y_top = format!("{hi:.2}");
    f.y_bot = format!("{lo:.2}");
    f
}