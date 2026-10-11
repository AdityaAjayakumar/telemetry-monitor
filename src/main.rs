mod logger;
mod parser;
mod plot;
mod preset;
mod source;
mod store;

use crossbeam_channel::bounded;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::{
    cell::RefCell,
    collections::HashSet,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};

slint::include_modules!();

struct App {
    store: store::Store,
    hidden: HashSet<String>,
    stop: Option<Arc<AtomicBool>>,
    log_join: Option<std::thread::JoinHandle<()>>,
    presets: Vec<(String, String)>,
}

fn update<T: Clone + 'static>(m: &VecModel<T>, v: Vec<T>) {
    if m.row_count() == v.len() {
        for (i, x) in v.into_iter().enumerate() {
            m.set_row_data(i, x);
        }
    } else {
        m.set_vec(v);
    }
}

fn list_ports() -> Vec<SharedString> {
    serialport::available_ports()
        .map(|v| v.into_iter().map(|p| p.port_name.into()).collect())
        .unwrap_or_default()
}

fn names_of(v: &[(String, String)]) -> Vec<SharedString> {
    v.iter().map(|(n, _)| n.as_str().into()).collect()
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let t0 = Instant::now();
    let (data_tx, data_rx) = bounded::<Arc<source::Sample>>(200_000);
    let log_slot: source::LogSlot = Arc::new(Mutex::new(None));

    // ---- presets ----
    let mut presets = preset::load_all();
    if presets.is_empty() {
        presets.push(("RTD rig".into(), "Tr1-8, Pr1-8:2".into()));
        let _ = preset::save_all(&presets);
    }
    let preset_names = Rc::new(VecModel::from(names_of(&presets)));
    let groups = Rc::new(VecModel::<SharedString>::default());
    let shared_preset: source::SharedPreset =
        Arc::new(RwLock::new(preset::Preset::default()));

    let app = Rc::new(RefCell::new(App {
        store: store::Store::new(200_000),
        hidden: HashSet::new(),
        stop: None,
        log_join: None,
        presets,
    }));

    let lines = Rc::new(VecModel::<SharedString>::default());
    let traces = Rc::new(VecModel::<Trace>::default());
    let bars = Rc::new(VecModel::<Bar>::default());
    let ports = Rc::new(VecModel::from(list_ports()));
    ui.set_lines(ModelRc::from(lines.clone()));
    ui.set_traces(ModelRc::from(traces.clone()));
    ui.set_bars(ModelRc::from(bars.clone()));
    ui.set_ports(ModelRc::from(ports.clone()));
    ui.set_preset_names(ModelRc::from(preset_names.clone()));
    ui.set_groups(ModelRc::from(groups.clone()));

    ui.on_refresh_ports({
        let ports = ports.clone();
        move || ports.set_vec(list_ports())
    });

    // ---- auto-refresh COM port list every 1.5 s ----
    let port_timer = Timer::default();
    port_timer.start(TimerMode::Repeated, Duration::from_millis(1500), {
        let ports = ports.clone();
        move || {
            let now = list_ports();
            let same = now.len() == ports.row_count()
                && now
                    .iter()
                    .enumerate()
                    .all(|(i, p)| ports.row_data(i).as_ref() == Some(p));
            if !same {
                ports.set_vec(now);
            }
        }
    });

    ui.on_connect({
        let app = app.clone();
        let weak = ui.as_weak();
        let data_tx = data_tx.clone();
        let slot = log_slot.clone();
        let shared_preset = shared_preset.clone();
        move |mode, target, baud| {
            let mut a = app.borrow_mut();
            if let Some(s) = a.stop.take() {
                s.store(true, Ordering::Relaxed);
            }
            let stop = Arc::new(AtomicBool::new(false));
            a.stop = Some(stop.clone());
            let target = target.to_string();
            let src = match mode {
                0 => source::Source::Serial {
                    port: target,
                    baud: baud as u32,
                },
                1 => source::Source::TcpListen(target),
                _ => source::Source::TcpConnect(target),
            };
            let w2 = weak.clone();
            let status: source::StatusFn = Arc::new(move |m: String| {
                let _ = w2.upgrade_in_event_loop(move |ui| ui.set_status(m.into()));
            });
            source::spawn(
                src,
                data_tx.clone(),
                slot.clone(),
                shared_preset.clone(),
                t0,
                stop,
                status,
            );
        }
    });

    ui.on_disconnect({
        let app = app.clone();
        move || {
            if let Some(s) = app.borrow_mut().stop.take() {
                s.store(true, Ordering::Relaxed);
            }
        }
    });

    ui.on_start_log({
        let app = app.clone();
        let slot = log_slot.clone();
        let weak = ui.as_weak();
        move |fmt, path| {
            let f = match fmt {
                0 => logger::Fmt::Csv,
                1 => logger::Fmt::Txt,
                _ => logger::Fmt::Log,
            };
            let p = std::path::Path::new(path.as_str()).with_extension(f.ext());
            match logger::start(&p.to_string_lossy(), f) {
                Ok((tx, jh)) => {
                    *slot.lock().unwrap() = Some(tx);
                    app.borrow_mut().log_join = Some(jh);
                    if let Some(ui) = weak.upgrade() {
                        ui.set_status(format!("Logging to {}", p.display()).into());
                    }
                }
                Err(e) => {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_status(format!("Log error: {e}").into());
                    }
                }
            }
        }
    });

    ui.on_stop_log({
        let app = app.clone();
        let slot = log_slot.clone();
        move || {
            *slot.lock().unwrap() = None; // drops Sender, logger flushes+exits
            if let Some(j) = app.borrow_mut().log_join.take() {
                let _ = j.join();
            }
        }
    });

    ui.on_browse_log({
        let weak = ui.as_weak();
        move |fmt| {
            let Some(ui) = weak.upgrade() else { return };
            let (label, ext) = match fmt {
                0 => ("CSV", "csv"),
                1 => ("Text", "txt"),
                _ => ("Log", "log"),
            };
            let cur = std::path::PathBuf::from(ui.get_log_path().as_str());
            let name = cur
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "telemetry_log".into());
            let mut dlg = rfd::FileDialog::new()
                .set_title("Choose log file")
                .add_filter(label, &[ext])
                .set_file_name(format!("{name}.{ext}"));
            if let Some(dir) = cur.parent().filter(|d| d.is_dir()) {
                dlg = dlg.set_directory(dir);
            }
            if let Some(p) = dlg.save_file() {
                ui.set_log_path(p.to_string_lossy().to_string().into());
            }
        }
    });

    // ---- channel checklist callbacks ----
    ui.on_toggle_trace({
        let app = app.clone();
        move |name| {
            let mut a = app.borrow_mut();
            let n = name.to_string();
            if !a.hidden.remove(&n) {
                a.hidden.insert(n);
            }
        }
    });

    ui.on_show_all({
        let app = app.clone();
        move || {
            app.borrow_mut().hidden.clear();
        }
    });

    ui.on_hide_all({
        let app = app.clone();
        move || {
            let mut a = app.borrow_mut();
            let names: HashSet<String> =
                a.store.series.iter().map(|s| s.name.clone()).collect();
            a.hidden = names;
        }
    });

    ui.on_only_group({
        let app = app.clone();
        move |prefix| {
            let mut a = app.borrow_mut();
            let p = prefix.to_lowercase();
            let hide: HashSet<String> = a
                .store
                .series
                .iter()
                .filter(|s| !s.name.to_lowercase().starts_with(&p))
                .map(|s| s.name.clone())
                .collect();
            a.hidden = hide;
        }
    });

    ui.on_clear({
        let app = app.clone();
        let lines = lines.clone();
        move || {
            app.borrow_mut().store.clear();
            lines.set_vec(vec![]);
        }
    });

    // ---- preset callbacks ----
    ui.on_select_preset({
        let app = app.clone();
        let weak = ui.as_weak();
        move |name| {
            let a = app.borrow();
            let found = a.presets.iter().find(|(n, _)| n.as_str() == name.as_str());
            if let (Some(ui), Some((n, s))) = (weak.upgrade(), found) {
                ui.set_preset_name(n.as_str().into());
                ui.set_preset_spec(s.as_str().into());
            }
        }
    });

    ui.on_apply_preset({
        let app = app.clone();
        let shared = shared_preset.clone();
        let groups = groups.clone();
        let weak = ui.as_weak();
        move |spec, strict| {
            let Some(ui) = weak.upgrade() else { return };
            match preset::Preset::parse(spec.as_str(), strict) {
                Ok(p) => {
                    let mut a = app.borrow_mut();
                    a.store.clear();
                    a.hidden.clear();
                    a.store.register(&p.names);
                    let g: Vec<SharedString> =
                        p.groups.iter().map(|g| g.as_str().into()).collect();
                    groups.set_vec(g);
                    ui.set_status(
                        format!("Preset applied: {} channels", p.names.len()).into(),
                    );
                    *shared.write().unwrap() = p;
                }
                Err(e) => ui.set_status(format!("Preset error: {e}").into()),
            }
        }
    });

    ui.on_free_mode({
        let app = app.clone();
        let shared = shared_preset.clone();
        let groups = groups.clone();
        let weak = ui.as_weak();
        move || {
            let mut a = app.borrow_mut();
            a.store.clear();
            a.hidden.clear();
            groups.set_vec(vec![]);
            *shared.write().unwrap() = preset::Preset::default();
            if let Some(ui) = weak.upgrade() {
                ui.set_status("Free mode: all channels accepted".into());
            }
        }
    });

    ui.on_save_preset({
        let app = app.clone();
        let preset_names = preset_names.clone();
        let weak = ui.as_weak();
        move |name, spec| {
            let Some(ui) = weak.upgrade() else { return };
            let name = name.trim().replace('=', "");
            if name.is_empty() {
                ui.set_status("Enter a preset name first".into());
                return;
            }
            if let Err(e) = preset::Preset::parse(spec.as_str(), false) {
                ui.set_status(format!("Preset error: {e}").into());
                return;
            }
            let mut a = app.borrow_mut();
            if let Some(i) = a.presets.iter().position(|(n, _)| *n == name) {
                a.presets[i].1 = spec.to_string();
            } else {
                a.presets.push((name.clone(), spec.to_string()));
            }
            let _ = preset::save_all(&a.presets);
            preset_names.set_vec(names_of(&a.presets));
            ui.set_status(format!("Saved preset '{name}'").into());
        }
    });

    ui.on_delete_preset({
        let app = app.clone();
        let preset_names = preset_names.clone();
        move |name| {
            let mut a = app.borrow_mut();
            a.presets.retain(|(n, _)| n.as_str() != name.as_str());
            let _ = preset::save_all(&a.presets);
            preset_names.set_vec(names_of(&a.presets));
        }
    });
        ui.on_detect_preset({
        let app = app.clone();
        let shared = shared_preset.clone();
        let groups = groups.clone();
        let weak = ui.as_weak();
        move |strict| {
            let Some(ui) = weak.upgrade() else { return };
            // start listening with no filtering
            {
                let mut a = app.borrow_mut();
                a.store.clear();
                a.hidden.clear();
            }
            *shared.write().unwrap() = preset::Preset::default();
            groups.set_vec(Vec::<SharedString>::new());
            ui.set_status("Learning channels from live data (4 s)...".into());

            // after 4 seconds, build the preset from what arrived
            Timer::single_shot(Duration::from_secs(4), {
                let app = app.clone();
                let shared = shared.clone();
                let groups = groups.clone();
                let weak = weak.clone();
                move || {
                    let Some(ui) = weak.upgrade() else { return };
                    let mut a = app.borrow_mut();
                    let names: Vec<String> = a
                        .store
                        .series
                        .iter()
                        .filter(|s| s.pts.len() >= 2)
                        .map(|s| s.name.clone())
                        .collect();
                    let spec = preset::spec_from_names(&names);
                    if spec.is_empty() {
                        ui.set_status(
                            "Nothing detected: connect first and make sure data is flowing"
                                .into(),
                        );
                        return;
                    }
                    match preset::Preset::parse(&spec, strict) {
                        Ok(p) => {
                            a.store.clear();
                            a.hidden.clear();
                            a.store.register(&p.names);
                            let g: Vec<SharedString> =
                                p.groups.iter().map(|g| g.as_str().into()).collect();
                            groups.set_vec(g);
                            *shared.write().unwrap() = p;
                            ui.set_preset_spec(spec.clone().into());
                            ui.set_status(format!("Detected: {spec}").into());
                        }
                        Err(e) => ui.set_status(format!("Detect failed: {e}").into()),
                    }
                }
            });
        }
    });

    // ---- 30 Hz UI tick: drain -> store -> redraw ----
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(33), {
        let app = app.clone();
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut a = app.borrow_mut();
            let mode = ui.get_view_mode();

            let mut fresh: Vec<SharedString> = Vec::new();
            for _ in 0..50_000 {
                let Ok(s) = data_rx.try_recv() else { break };
                a.store.push(&s);
                if mode == 0 {
                    fresh.push(format!("{:9.3}  {}", s.t, s.raw).into());
                }
            }

            if mode == 0 {
                let skip = fresh.len().saturating_sub(500);
                for l in fresh.into_iter().skip(skip) {
                    lines.push(l);
                }
                while lines.row_count() > 2000 {
                    lines.remove(0);
                }
                if ui.get_autoscroll() {
                    ui.invoke_scroll_bottom();
                }
                return;
            }

            // everything is plotted; hide channels with the side checklist
            let win = ui.get_window_s().max(1) as f64;
            let (w, h) = (ui.get_plot_w(), ui.get_plot_h());
            let f = match mode {
                1 => plot::lines(&a.store, &a.hidden, win, w, h),
                _ => plot::boxes(&a.store, &a.hidden, win, w, h),
            };
            update(&traces, f.traces);
            update(&bars, f.bars);
            ui.set_y_top(f.y_top.into());
            ui.set_y_bot(f.y_bot.into());
        }
    });

    ui.run()
}