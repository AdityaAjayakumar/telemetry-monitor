use crate::preset::Preset;

pub fn parse_line(line: &str, p: &Preset) -> Vec<(String, f64)> {
    if line.contains(':') || line.contains('=') {
        parse_kv(line, p)
    } else {
        parse_csv(line, p)
    }
}

// "Pr1,168.750,150.000" -> Pr1=168.75, Pr1.2=150
fn parse_csv(line: &str, p: &Preset) -> Vec<(String, f64)> {
    let mut it = line.split(',').map(str::trim);
    let Some(name) = it.next() else { return vec![] };
    if name.is_empty() || name.parse::<f64>().is_ok() {
        return vec![];
    }
    let vals: Vec<f64> = it.map_while(|t| t.parse().ok()).collect();
    if vals.is_empty() {
        return vec![];
    }
    let (canon, keep) = match p.resolve(name) {
        Some((c, n)) => (c.to_string(), n),
        None if p.strict => return vec![],
        None => (name.to_string(), usize::MAX),
    };
    vals.into_iter()
        .take(keep)
        .enumerate()
        .map(|(i, v)| {
            let n = if i == 0 { canon.clone() } else { format!("{canon}.{}", i + 1) };
            (n, v)
        })
        .collect()
}

// "Tr1:23.5,Tr2=24.1"
fn parse_kv(line: &str, p: &Preset) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let seps = |c: char| c == ',' || c == ';' || c.is_whitespace();
    for tok in line.split(seps).filter(|t| !t.is_empty()) {
        let Some((k, v)) = tok.split_once(|c: char| c == ':' || c == '=') else {
            continue;
        };
        let Ok(x) = v.parse::<f64>() else { continue };
        match p.resolve(k) {
            Some((c, _)) => out.push((c.to_string(), x)),
            None if p.strict => {}
            None => out.push((k.to_string(), x)),
        }
    }
    out
}