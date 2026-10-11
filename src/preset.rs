use std::{collections::HashMap, path::PathBuf};

#[derive(Clone, Default)]
pub struct Preset {
    pub strict: bool,
    pub names: Vec<String>,  // ordered channel names
    pub groups: Vec<String>, // prefixes, for quick-select buttons
    map: HashMap<String, (String, usize)>, // lower -> (canon, nvals)
}

impl Preset {
    /// "Tr1-7, Pr1-4:2, Acc1-9"
    pub fn parse(spec: &str, strict: bool) -> Result<Self, String> {
        let mut p = Preset { strict, ..Default::default() };
        let toks = spec
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .filter(|t| !t.is_empty());
        for tok in toks {
            let (body, nvals) = match tok.split_once(':') {
                Some((b, n)) => {
                    let n = n
                        .parse::<usize>()
                        .map_err(|_| format!("bad count in '{tok}'"))?;
                    (b, n.clamp(1, 16))
                }
                None => (tok, 1),
            };
            let cut = body
                .find(|c: char| c.is_ascii_digit())
                .unwrap_or(body.len());
            let (prefix, range) = body.split_at(cut);
            if prefix.is_empty() {
                return Err(format!("'{tok}' needs a name before the number"));
            }
            if !p.groups.iter().any(|g| g == prefix) {
                p.groups.push(prefix.to_string());
            }
            if range.is_empty() {
                p.add(prefix.to_string(), nvals);
                continue;
            }
            let (a, b) = range.split_once('-').unwrap_or((range, range));
            let a = a.parse::<u32>().map_err(|_| format!("bad range in '{tok}'"))?;
            let b = b.parse::<u32>().map_err(|_| format!("bad range in '{tok}'"))?;
            if b < a || b - a > 999 {
                return Err(format!("bad range in '{tok}'"));
            }
            for n in a..=b {
                p.add(format!("{prefix}{n}"), nvals);
            }
        }
        if p.names.is_empty() {
            return Err("no channels in preset".into());
        }
        Ok(p)
    }

    fn add(&mut self, canon: String, nvals: usize) {
        let key = canon.to_ascii_lowercase();
        if self.map.contains_key(&key) {
            return;
        }
        self.map.insert(key, (canon.clone(), nvals));
        self.names.push(canon.clone());
        for i in 2..=nvals {
            self.names.push(format!("{canon}.{i}"));
        }
    }

    pub fn resolve(&self, name: &str) -> Option<(&str, usize)> {
        self.map
            .get(&name.to_ascii_lowercase())
            .map(|(c, n)| (c.as_str(), *n))
    }
}

/// Build a preset string like "Tr1-8, Pr1-8:2" from channel names seen
/// in live data ("Tr1", "Pr1", "Pr1.2", ...).
pub fn spec_from_names(names: &[String]) -> String {
    // 1) base name -> how many values per line were seen
    let mut bases: Vec<(String, usize)> = Vec::new();
    for n in names {
        let (base, k) = match n.rsplit_once('.') {
            Some((b, k)) if k.parse::<usize>().map_or(false, |k| k >= 2) => {
                (b.to_string(), k.parse::<usize>().unwrap_or(2))
            }
            _ => (n.clone(), 1),
        };
        if let Some(i) = bases.iter().position(|(b, _)| *b == base) {
            bases[i].1 = bases[i].1.max(k);
        } else {
            bases.push((base, k));
        }
    }

    // 2) group by prefix: (prefix, numbers, values per line)
    let mut groups: Vec<(String, Vec<u32>, usize)> = Vec::new();
    let mut bare: Vec<String> = Vec::new();
    for (base, nv) in &bases {
        let cut = base
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(base.len());
        let (prefix, num) = base.split_at(cut);
        match num.parse::<u32>() {
            Ok(n) if !prefix.is_empty() => {
                let g = match groups.iter().position(|(p, _, _)| p.as_str() == prefix) {
                    Some(i) => i,
                    None => {
                        groups.push((prefix.to_string(), vec![], 1));
                        groups.len() - 1
                    }
                };
                groups[g].1.push(n);
                groups[g].2 = groups[g].2.max(*nv);
            }
            _ => {
                // names with no digits can be expressed; others cannot
                if !base.chars().any(|c| c.is_ascii_digit()) {
                    bare.push(if *nv > 1 {
                        format!("{base}:{nv}")
                    } else {
                        base.clone()
                    });
                }
            }
        }
    }

    // 3) compress runs of numbers into ranges
    let mut out: Vec<String> = Vec::new();
    for (prefix, mut nums, nv) in groups {
        nums.sort_unstable();
        nums.dedup();
        let suffix = if nv > 1 { format!(":{nv}") } else { String::new() };
        let mut i = 0;
        while i < nums.len() {
            let mut j = i;
            while j + 1 < nums.len() && nums[j + 1] == nums[j] + 1 {
                j += 1;
            }
            if j == i {
                out.push(format!("{prefix}{}{suffix}", nums[i]));
            } else {
                out.push(format!("{prefix}{}-{}{suffix}", nums[i], nums[j]));
            }
            i = j + 1;
        }
    }
    out.extend(bare);
    out.join(", ")
}

// ---- saved presets: one "name=spec" per line ----

fn path() -> PathBuf {
    let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("telemetry-monitor");
    std::fs::create_dir_all(&p).ok();
    p.push("presets.txt");
    p
}

pub fn load_all() -> Vec<(String, String)> {
    std::fs::read_to_string(path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            let (n, s) = l.split_once('=')?;
            Some((n.trim().to_string(), s.trim().to_string()))
        })
        .collect()
}

pub fn save_all(v: &[(String, String)]) -> std::io::Result<()> {
    let mut s = String::new();
    for (n, sp) in v {
        s.push_str(&format!("{n}={sp}\n"));
    }
    std::fs::write(path(), s)
}