use crate::store::{Series, Store};
use crate::{Bar, Dot, Reading, Trace};
use slint::Color;
use std::{
    collections::{HashSet, VecDeque},
    fmt::Write,
};

const MIN_SPAN: f64 = 0.5; // never zoom in past half a second

// ======================================================================
//  View state: what part of the data is on screen
// ======================================================================
#[derive(Clone)]
pub struct View {
    /// time axis fits all data (true) or is zoomed (false)
    pub auto: bool,
    /// visible seconds while zoomed
    pub span: f64,
    /// zoomed view sticks to the newest data
    pub follow: bool,
    /// right edge (absolute seconds) while zoomed and not following
    pub end: f64,
    /// manual value range; None = fit to what is visible
    pub y_manual: Option<(f64, f64)>,
    // what the last frame actually showed (gestures work from these)
    pub t0: f64,
    pub t1: f64,
    pub y_lo: f64,
    pub y_hi: f64,
    pub data_t0: f64,
    pub data_t1: f64,
}

impl Default for View {
    fn default() -> Self {
        View {
            auto: true,
            span: 30.0,
            follow: true,
            end: 0.0,
            y_manual: None,
            t0: 0.0,
            t1: 1.0,
            y_lo: 0.0,
            y_hi: 1.0,
            data_t0: 0.0,
            data_t1: 1.0,
        }
    }
}

impl View {
    pub fn reset(&mut self) {
        self.auto = true;
        self.follow = true;
        self.y_manual = None;
    }

    pub fn is_auto(&self) -> bool {
        self.auto && self.y_manual.is_none()
    }

    pub fn label(&self) -> &'static str {
        if !self.auto && !self.follow {
            "Paused on older data"
        } else if !self.auto {
            "Zoomed in, following live"
        } else if self.y_manual.is_some() {
            "Value axis zoomed"
        } else {
            "Fitting all data"
        }
    }

    /// time range to draw this frame
    pub fn resolve_x(&self, d0: f64, d1: f64) -> (f64, f64) {
        let (mut t0, t1) = if self.auto {
            (d0, d1)
        } else if self.follow {
            (d1 - self.span, d1)
        } else {
            (self.end - self.span, self.end)
        };
        if t1 - t0 < MIN_SPAN {
            t0 = t1 - MIN_SPAN;
        }
        (t0, t1)
    }

    pub fn record(&mut self, f: &Frame, d0: f64, d1: f64) {
        self.t0 = f.t0;
        self.t1 = f.t1;
        self.y_lo = f.y_lo;
        self.y_hi = f.y_hi;
        self.data_t0 = d0;
        self.data_t1 = d1;
    }

    fn go_manual(&mut self) {
        if self.auto {
            self.auto = false;
            self.span = (self.t1 - self.t0).max(MIN_SPAN);
            self.follow = true;
            self.end = self.t1;
        }
    }

    fn set_range(&mut self, t1: f64, span: f64) {
        let mut t1 = t1;
        let min_t1 = self.data_t0 + span; // keep the window inside the data
        if t1 < min_t1 {
            t1 = min_t1;
        }
        if t1 >= self.data_t1 - 1e-6 {
            self.follow = true;
            self.end = self.data_t1;
        } else {
            self.follow = false;
            self.end = t1;
        }
        self.span = span;
    }

    /// zoom the time axis around fraction fx (0..1 across the plot).
    /// steps > 0 zooms in, steps < 0 zooms out.
    pub fn zoom_x(&mut self, fx: f64, steps: f64) {
        self.go_manual();
        let full = (self.data_t1 - self.data_t0).max(MIN_SPAN);
        let cur_t1 = if self.follow { self.data_t1 } else { self.end };
        let cur_span = self.span;
        let anchor = cur_t1 - cur_span + fx * cur_span;
        let new_span = (cur_span * 0.85f64.powf(steps)).clamp(MIN_SPAN, full);
        if new_span >= full * 0.999 {
            // zoomed all the way out: back to fitting everything
            self.auto = true;
            self.follow = true;
            return;
        }
        self.set_range(anchor + (1.0 - fx) * new_span, new_span);
    }

    /// drag left/right (dxf = fraction of plot width moved)
    pub fn pan_x(&mut self, dxf: f64) {
        if self.auto {
            return;
        }
        let cur_t1 = if self.follow { self.data_t1 } else { self.end };
        self.set_range(cur_t1 - dxf * self.span, self.span);
    }

    /// zoom the value axis around fraction fy (0 = top)
    pub fn zoom_y(&mut self, fy: f64, steps: f64) {
        let (lo, hi) = self.y_manual.unwrap_or((self.y_lo, self.y_hi));
        let span = hi - lo;
        let anchor = hi - fy * span;
        let new_span = (span * 0.85f64.powf(steps)).max(1e-9);
        let new_hi = anchor + fy * new_span;
        self.y_manual = Some((new_hi - new_span, new_hi));
    }

    /// drag up/down; only moves the view once the value axis is zoomed
    pub fn pan_y(&mut self, dyf: f64) {
        if let Some((lo, hi)) = self.y_manual {
            let d = dyf * (hi - lo);
            self.y_manual = Some((lo + d, hi + d));
        }
    }
}

// ======================================================================
//  Frame: everything the UI needs for one redraw
// ======================================================================
#[derive(Default)]
pub struct Frame {
    pub traces: Vec<Trace>,
    pub bars: Vec<Bar>,
    pub dots: Vec<Dot>,
    pub x_ticks: Vec<String>,
    pub y_ticks: Vec<String>,
    pub t0: f64,
    pub t1: f64,
    pub y_lo: f64,
    pub y_hi: f64,
}

pub struct Hover {
    pub readings: Vec<Reading>,
    pub dots: Vec<Dot>,
    pub more: String,
    pub time: String,
    pub value: String,
}

// ======================================================================
//  helpers
// ======================================================================
fn nat_key(name: &str) -> (String, u32) {
    let digits = name.chars().rev().take_while(|c| c.is_ascii_digit()).count();
    let (a, b) = name.split_at(name.len() - digits);
    (a.to_string(), b.parse().unwrap_or(0))
}

/// strong mid-tone colours that stay readable on pale chart paper
fn color(i: usize) -> Color {
    let h = (i as f32 * 137.508) % 360.0;
    let (s, v) = (0.78f32, 0.72f32);
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

fn ordered(store: &Store) -> Vec<(usize, &Series)> {
    let mut v: Vec<_> = store.series.iter().enumerate().collect();
    if !store.keep_order {
        v.sort_by_key(|(_, s)| nat_key(&s.name));
    }
    v
}

/// earliest and latest sample time over the visible channels
pub fn data_range(store: &Store, hidden: &HashSet<String>) -> (f64, f64) {
    let (mut t0, mut t1) = (f64::MAX, f64::MIN);
    for s in &store.series {
        if hidden.contains(&s.name) {
            continue;
        }
        if let (Some(a), Some(b)) = (s.pts.front(), s.pts.back()) {
            t0 = t0.min(a.0);
            t1 = t1.max(b.0);
        }
    }
    if t0 > t1 {
        return (0.0, MIN_SPAN);
    }
    if t1 - t0 < MIN_SPAN {
        t0 = t1 - MIN_SPAN;
    }
    (t0, t1)
}

fn fmt_val(v: f64, range: f64) -> String {
    let d = if range < 0.05 {
        4
    } else if range < 0.5 {
        3
    } else if range < 50.0 {
        2
    } else if range < 500.0 {
        1
    } else {
        0
    };
    format!("{:.*}", d, v)
}

fn fmt_ago(s: f64, span: f64) -> String {
    if s < 0.05 {
        "now".into()
    } else if span < 20.0 {
        format!("-{:.1}s", s)
    } else if s < 120.0 {
        format!("-{:.0}s", s)
    } else if s < 7200.0 {
        format!("-{}m{:02}s", (s / 60.0) as u64, (s % 60.0) as u64)
    } else {
        format!("-{}h{:02}m", (s / 3600.0) as u64, ((s % 3600.0) / 60.0) as u64)
    }
}

fn x_ticks(t0: f64, t1: f64, now: f64) -> Vec<String> {
    let span = t1 - t0;
    (0..7)
        .map(|i| fmt_ago(now - (t0 + span * i as f64 / 6.0), span))
        .collect()
}

fn y_ticks(lo: f64, hi: f64) -> Vec<String> {
    (0..5)
        .map(|i| fmt_val(hi - (hi - lo) * i as f64 / 4.0, hi - lo))
        .collect()
}

fn range_or_default(lo: f64, hi: f64) -> (f64, f64) {
    let (mut lo, mut hi) = if lo > hi { (0.0, 1.0) } else { (lo, hi) };
    if (hi - lo).abs() < 1e-9 {
        hi = lo + 1.0;
    }
    let pad = (hi - lo) * 0.06;
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
            has_tip: false,
            tip_x: 0.0,
            tip_y: 0.0,
        })
        .collect()
}

/// min/max per pixel column inside [t0, t1], keeping time order in a column
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
    let end = pts.partition_point(|p| p.0 <= t1);
    let span = (t1 - t0).max(1e-9);
    let mut out = Vec::with_capacity(cols * 2);
    let (mut col, mut idx) = (usize::MAX, 0usize);
    let (mut mn, mut mx) = ((0usize, 0.0f64), (0usize, 0.0f64));
    for &(t, v) in pts.range(start..end.max(start)) {
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

// ======================================================================
//  line plot
// ======================================================================
pub fn lines(
    store: &Store,
    hidden: &HashSet<String>,
    t0: f64,
    t1: f64,
    now: f64,
    y_manual: Option<(f64, f64)>,
    w: f32,
    h: f32,
) -> Frame {
    let span = (t1 - t0).max(1e-9);
    let cols = (w.max(50.0)) as usize;
    let xs = w / cols as f32;
    let list = ordered(store);

    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    let mut data = Vec::new();
    for (_, s) in &list {
        let d = if hidden.contains(&s.name) {
            vec![]
        } else {
            decimate(&s.pts, t0, t1, cols)
        };
        if y_manual.is_none() {
            for &(_, v) in &d {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        data.push(d);
    }
    let (lo, hi) = match y_manual {
        Some(r) => r,
        None => range_or_default(lo, hi),
    };
    let ymap = |v: f64| ((1.0 - (v - lo) / (hi - lo)) as f32) * h;

    let mut traces = legend(&list, hidden);
    for ((tr, d), (_, s)) in traces.iter_mut().zip(&data).zip(&list) {
        let mut cmd = String::with_capacity(d.len() * 12);
        for (k, (x, v)) in d.iter().enumerate() {
            let _ = write!(
                cmd,
                "{}{:.1} {:.1} ",
                if k == 0 { "M" } else { "L" },
                x * xs,
                ymap(*v)
            );
        }
        tr.commands = cmd.into();

        // pen tip: only when this channel's newest sample is at the live edge
        if let Some(&(tl, vl)) = s.pts.back() {
            let fresh = t1 - tl <= (span * 0.03).max(1.5);
            if !hidden.contains(&s.name) && tl >= t0 && tl <= t1 && fresh {
                tr.has_tip = true;
                tr.tip_x = (((tl - t0) / span) as f32) * w;
                tr.tip_y = ymap(vl);
            }
        }
    }

    Frame {
        traces,
        x_ticks: x_ticks(t0, t1, now),
        y_ticks: y_ticks(lo, hi),
        t0,
        t1,
        y_lo: lo,
        y_hi: hi,
        ..Default::default()
    }
}

/// values under the pointer (line mode)
pub fn hover(
    store: &Store,
    hidden: &HashSet<String>,
    view: &View,
    fx: f64,
    fy: f64,
    w: f32,
    h: f32,
    line_mode: bool,
) -> Hover {
    let span = (view.t1 - view.t0).max(1e-9);
    let range = (view.y_hi - view.y_lo).max(1e-12);
    let t = view.t0 + fx.clamp(0.0, 1.0) * span;
    let v = view.y_hi - fy.clamp(0.0, 1.0) * range;
    let mut out = Hover {
        readings: vec![],
        dots: vec![],
        more: String::new(),
        time: fmt_ago(view.data_t1 - t, span),
        value: fmt_val(v, range),
    };
    if !line_mode {
        return out;
    }
    let ymap = |val: f64| ((1.0 - (val - view.y_lo) / range) as f32) * h;
    let mut total = 0usize;
    for (i, s) in ordered(store) {
        if hidden.contains(&s.name) || s.pts.is_empty() {
            continue;
        }
        // nearest sample to the pointer's time
        let k = s.pts.partition_point(|p| p.0 < t);
        let cand = [k.checked_sub(1), Some(k)];
        let best = cand
            .iter()
            .flatten()
            .filter_map(|&j| s.pts.get(j).copied())
            .min_by(|a, b| {
                (a.0 - t)
                    .abs()
                    .partial_cmp(&(b.0 - t).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        let Some((ts, val)) = best else { continue };
        if ts < view.t0 || ts > view.t1 || (ts - t).abs() > span * 0.25 {
            continue;
        }
        total += 1;
        if out.readings.len() < 16 {
            out.readings.push(Reading {
                name: s.name.clone().into(),
                color: color(i),
                value: format!("{:.2}", val).into(),
            });
            out.dots.push(Dot {
                x: (((ts - view.t0) / span) as f32) * w,
                y: ymap(val),
                color: color(i),
            });
        }
    }
    if total > 16 {
        out.more = format!("+{} more", total - 16);
    }
    out
}

// ======================================================================
//  box and whisker (Tukey, like matplotlib / numpy percentiles)
// ======================================================================
fn pct(v: &[f64], p: f64) -> f64 {
    let pos = (v.len() - 1) as f64 * p;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    v[lo] + (v[hi] - v[lo]) * (pos - lo as f64)
}

struct Stats {
    idx: usize,
    name: String,
    n: usize,
    step: usize,
    min: f64,
    max: f64,
    q1: f64,
    med: f64,
    q3: f64,
    mean: f64,
    wlo: f64,
    whi: f64,
    outliers: Vec<f64>,
}

pub fn boxes(
    store: &Store,
    hidden: &HashSet<String>,
    t0: f64,
    t1: f64,
    now: f64,
    y_manual: Option<(f64, f64)>,
    w: f32,
    h: f32,
) -> Frame {
    let list = ordered(store);
    let mut f = Frame {
        traces: legend(&list, hidden),
        ..Default::default()
    };

    let mut stats: Vec<Stats> = Vec::new();
    for (i, s) in list.iter().filter(|(_, s)| !hidden.contains(&s.name)) {
        let start = s.pts.partition_point(|p| p.0 < t0);
        let end = s.pts.partition_point(|p| p.0 <= t1).max(start);
        let n = end - start;
        if n == 0 {
            continue;
        }
        let step = (n / 4000).max(1);
        let mut v: Vec<f64> = s.pts.range(start..end).step_by(step).map(|p| p.1).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let q1 = pct(&v, 0.25);
        let med = pct(&v, 0.5);
        let q3 = pct(&v, 0.75);
        let iqr = q3 - q1;
        let (flo, fhi) = (q1 - 1.5 * iqr, q3 + 1.5 * iqr);
        let wlo = v.iter().copied().find(|x| *x >= flo).unwrap_or(v[0]);
        let whi = v.iter().rev().copied().find(|x| *x <= fhi).unwrap_or(v[v.len() - 1]);
        let outliers: Vec<f64> = v.iter().copied().filter(|x| *x < wlo || *x > whi).collect();
        stats.push(Stats {
            idx: *i,
            name: s.name.clone(),
            n,
            step,
            min: v[0],
            max: v[v.len() - 1],
            q1,
            med,
            q3,
            mean: v.iter().sum::<f64>() / v.len() as f64,
            wlo,
            whi,
            outliers,
        });
    }

    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for s in &stats {
        lo = lo.min(s.min);
        hi = hi.max(s.max);
    }
    let (lo, hi) = match y_manual {
        Some(r) => r,
        None => range_or_default(lo, hi),
    };
    let range = hi - lo;
    let ymap = |v: f64| ((1.0 - (v - lo) / range) as f32) * h;
    let slot = w / stats.len().max(1) as f32;

    for (k, s) in stats.iter().enumerate() {
        let x = (k as f32 + 0.5) * slot;
        let fv = |v: f64| fmt_val(v, range.min(s.max - s.min).max(1e-9));
        let rows: Vec<(&str, String)> = vec![
            ("n", s.n.to_string()),
            ("max", fv(s.max)),
            ("upper whisker", fv(s.whi)),
            ("Q3 (75%)", fv(s.q3)),
            ("median", fv(s.med)),
            ("Q1 (25%)", fv(s.q1)),
            ("lower whisker", fv(s.wlo)),
            ("min", fv(s.min)),
            ("IQR", fv(s.q3 - s.q1)),
            ("mean", fv(s.mean)),
            ("outliers", s.outliers.len().to_string()),
        ];
        let mut tip = rows.iter().map(|r| r.0.to_string()).collect::<Vec<_>>().join("\n");
        let mut tip_vals = rows.iter().map(|r| r.1.clone()).collect::<Vec<_>>().join("\n");
        if s.step > 1 {
            tip.push_str("\nsampled");
            tip_vals.push_str(&format!("\nevery {}th", s.step));
        }
        f.bars.push(Bar {
            x,
            w: (slot * 0.5).clamp(8.0, 64.0),
            y_lo: ymap(s.wlo),
            y_hi: ymap(s.whi),
            y_a: ymap(s.q3),
            y_b: ymap(s.q1),
            y_mid: ymap(s.med),
            color: color(s.idx),
            label: s.name.clone().into(),
            tip: tip.into(),
            tip_vals: tip_vals.into(),
        });
        // outliers drawn as hollow dots (at most 40 per box)
        let stride = (s.outliers.len() / 40).max(1);
        for v in s.outliers.iter().step_by(stride) {
            f.dots.push(Dot {
                x,
                y: ymap(*v),
                color: color(s.idx),
            });
        }
    }

    f.x_ticks = x_ticks(t0, t1, now);
    f.y_ticks = y_ticks(lo, hi);
    f.t0 = t0;
    f.t1 = t1;
    f.y_lo = lo;
    f.y_hi = hi;
    f
}