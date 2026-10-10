use crate::source::Sample;
use std::collections::{HashMap, VecDeque};

pub struct Series {
    pub name: String,
    pub pts: VecDeque<(f64, f64)>,
}

pub struct Store {
    pub series: Vec<Series>,
    idx: HashMap<String, usize>,
    cap: usize,
    pub keep_order: bool,
}

impl Store {
    pub fn new(cap: usize) -> Self {
        Self {
            series: vec![],
            idx: HashMap::new(),
            cap,
            keep_order: false,
        }
    }

    pub fn clear(&mut self) {
        self.series.clear();
        self.idx.clear();
        self.keep_order = false;
    }

    /// Pre-create channels (used by presets) so they show up in the
    /// legend in preset order before any data arrives.
    pub fn register(&mut self, names: &[String]) {
        for n in names {
            if !self.idx.contains_key(n) {
                let i = self.series.len();
                self.series.push(Series {
                    name: n.clone(),
                    pts: VecDeque::new(),
                });
                self.idx.insert(n.clone(), i);
            }
        }
        self.keep_order = true;
    }

    pub fn push(&mut self, s: &Sample) {
        for (k, v) in &s.vals {
            let i = match self.idx.get(k) {
                Some(&i) => i,
                None => {
                    let i = self.series.len();
                    self.series.push(Series {
                        name: k.clone(),
                        pts: VecDeque::new(),
                    });
                    self.idx.insert(k.clone(), i);
                    i
                }
            };
            let sr = &mut self.series[i];
            if sr.pts.len() >= self.cap {
                sr.pts.pop_front();
            }
            sr.pts.push_back((s.t, *v));
        }
    }

    pub fn t_max(&self) -> f64 {
        self.series
            .iter()
            .filter_map(|s| s.pts.back().map(|p| p.0))
            .fold(0.0, f64::max)
    }
}