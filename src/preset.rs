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