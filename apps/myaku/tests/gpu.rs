use std::sync::Arc;

use aizome_kit::Size;
use aizome_kit::sidestep_ui::{Key, Modifiers, NamedKey};
use aizome_kit::testing::Harness;
use aizome_kit::theme::{Appearance, ThemeChoice};
use myaku::app::{Msg, Myaku, Request, RowRef, Sel, Show};
use myaku::model::{Frame, Gpu, GpuUsage, GpuUse, Group, Kind, Lens, Process, Usage};

fn frame(tick: u64) -> Frame {
    let mut f = myaku::fixture::frame(300, tick);
    f.gpus = vec![
        Gpu {
            name: "AMD Radeon Graphics".into(),
            pci: "0000:78:00.0".into(),
            integrated: true,
            busy: Some(18.0),
            vram_used: Some(22 << 20),
            vram_total: Some(2 << 30),
            ..Default::default()
        },
        Gpu {
            name: "NVIDIA GeForce RTX 5090".into(),
            pci: "0000:01:00.0".into(),
            busy: Some(3.0),
            vram_used: Some(2941 << 20),
            vram_total: Some(32 << 30),
            ..Default::default()
        },
    ];
    let client = |device, busy, mib: u64| GpuUse {
        device,
        usage: GpuUsage { busy: Some(busy), vram: Some(mib << 20), ..Default::default() },
    };
    f.processes = vec![
        Process {
            pid: 10,
            start_ticks: 1,
            name: "aizome".into(),
            uid: 1000,
            readable: true,
            group: 0,
            usage: Usage { cpu: 1.5, ..Default::default() },
            gpus: vec![client(0, 18.0, 6), client(1, 2.0, 876)],
            ..Default::default()
        },
        Process {
            pid: 20,
            start_ticks: 2,
            name: "koe".into(),
            uid: 1000,
            readable: true,
            group: 1,
            gpus: vec![client(1, 3.0, 355)],
            ..Default::default()
        },
        Process {
            pid: 21,
            start_ticks: 3,
            name: "koe helper".into(),
            uid: 1000,
            readable: true,
            group: 1,
            gpus: vec![client(1, 4.0, 10)],
            ..Default::default()
        },
    ];
    for p in &mut f.processes {
        for gpu in &p.gpus {
            p.usage.graphics.add(&gpu.usage);
        }
    }
    f.groups = vec![
        Group {
            key: (10, 1),
            name: "Aizome".into(),
            kind: Some(Kind::Service),
            own: true,
            members: vec![0],
            ..Default::default()
        },
        Group {
            key: (20, 2),
            name: "Koe".into(),
            kind: Some(Kind::App),
            own: true,
            members: vec![1, 2],
            ..Default::default()
        },
    ];
    for g in &mut f.groups {
        for &i in &g.members {
            g.usage.add(&f.processes[i].usage);
        }
    }
    f
}

fn start() -> Harness<Myaku> {
    let (signals, _) = Myaku::recorder();
    Harness::new(Myaku::with_frame(
        Arc::new(frame(1)),
        signals,
        Request { lens: Some(Lens::Gpu), ..Default::default() },
    ))
}

fn choose(h: &mut Harness<Myaku>, index: usize) {
    let w = h.window();
    let rect = h.find_id(w, "gpu-selector").expect("GPU selector");
    h.click(w, rect.center());
    let popup = h.popups(w).last().expect("GPU menu").0;
    h.popup_keyboard(popup, Key::Named(NamedKey::Home), Modifiers::NONE);
    for _ in 0..index {
        h.popup_keyboard(popup, Key::Named(NamedKey::ArrowDown), Modifiers::NONE);
    }
    h.popup_keyboard(popup, Key::Named(NamedKey::Enter), Modifiers::NONE);
}

#[test]
fn selecting_a_gpu_updates_values_groups_sorting_and_inspector() {
    let mut h = start();
    let w = h.window();
    assert_eq!(h.program().rows[0], RowRef::Group(0)); // AMD 18 + NVIDIA 2.
    choose(&mut h, 2);
    assert_eq!(h.program().gpu.as_deref(), Some("0000:01:00.0"));
    assert_eq!(h.program().rows[0], RowRef::Group(1)); // NVIDIA 3 + 4.
    assert_eq!(h.program().frame.groups[1].usage.graphics.vram, Some(365 << 20));
    assert_eq!(h.program().frame.processes[0].usage.graphics.vram, Some(876 << 20));
    assert_eq!(h.program().frame.processes[0].usage.cpu, 1.5);
    h.send(Msg::Select(Sel::Group((20, 2))));
    h.send(Msg::Inspector(true));
    assert!(h.texts(w).iter().any(|s| s == "365 MiB"));
    assert!(h.texts(w).iter().any(|s| s.contains("GRAPHICS · NVIDIA")));
    h.send(Msg::Toggle((20, 2)));
    assert_eq!(h.program().rows[1], RowRef::Process(2, true));
    h.send(Msg::Show(Show::All));
    assert_eq!(h.program().rows[0], RowRef::Process(2, false));
    choose(&mut h, 1);
    assert_eq!(*h.program().rows, vec![RowRef::Process(0, false)]);
    assert_eq!(h.program().frame.groups[1].usage.graphics.vram, Some(0));
    assert_eq!(h.program().sel, Some(Sel::Group((20, 2))));
    choose(&mut h, 0);
    assert_eq!(h.program().frame.processes[0].usage.graphics.vram, Some(882 << 20));
}

#[test]
fn filtering_keeps_idle_clients_and_respects_search_and_process_views() {
    let mut h = start();
    let mut f = frame(2);
    f.processes[1].gpus[0].usage.busy = Some(0.0); // Koe holds memory while idle.
    f.processes[2].gpus.clear(); // Its CPU helper has no GPU context.
    h.send(Msg::Frame(Arc::new(f)));
    choose(&mut h, 2);
    assert_eq!(h.program().rows.len(), 2);
    h.send(Msg::Toggle((20, 2)));
    assert!(h.program().rows.contains(&RowRef::Process(1, true)));
    assert!(!h.program().rows.contains(&RowRef::Process(2, true)));
    assert!(h.texts(h.window()).iter().any(|s| s == "1 of 2 processes"));
    h.send(Msg::Search("helper".into()));
    assert!(h.program().rows.is_empty());
    h.send(Msg::Search("koe".into()));
    assert_eq!(*h.program().rows, vec![RowRef::Group(1), RowRef::Process(1, true)]);
    h.send(Msg::Show(Show::All));
    assert_eq!(*h.program().rows, vec![RowRef::Process(1, false)]);
    h.send(Msg::Show(Show::Mine));
    assert_eq!(*h.program().rows, vec![RowRef::Process(1, false)]);
    h.send(Msg::Lens(Lens::Cpu));
    assert_eq!(h.program().rows.len(), 2);
    h.send(Msg::Lens(Lens::Gpu));
    assert_eq!(*h.program().rows, vec![RowRef::Process(1, false)]);
    h.send(Msg::Search(String::new()));
    choose(&mut h, 1);
    h.send(Msg::Show(Show::Apps));
    assert_eq!(*h.program().rows, vec![RowRef::Group(0)]);
    choose(&mut h, 0);
    h.send(Msg::Show(Show::All));
    assert_eq!(h.program().rows.len(), 3);
}

#[test]
fn selection_survives_readings_reordering_and_other_lenses() {
    let mut h = start();
    choose(&mut h, 2);
    h.send(Msg::Lens(Lens::Cpu));
    let mut f = frame(2);
    f.gpus.swap(0, 1);
    for p in &mut f.processes {
        for g in &mut p.gpus {
            g.device = 1 - g.device;
        }
    }
    h.send(Msg::Frame(Arc::new(f)));
    h.send(Msg::Lens(Lens::Gpu));
    assert_eq!(h.program().gpu.as_deref(), Some("0000:01:00.0"));
    assert_eq!(h.program().frame.processes[0].usage.graphics.vram, Some(876 << 20));
    let mut gone = frame(3);
    gone.gpus.pop();
    h.send(Msg::Frame(Arc::new(gone)));
    assert_eq!(h.program().gpu, None);
    assert!(h.find_id(h.window(), "gpu-selector").is_none());
}

#[test]
fn unknown_measurements_stay_unknown_and_sort_last() {
    let mut h = start();
    let mut f = frame(2);
    f.gpus[1].busy = None;
    f.gpus[1].vram_used = None;
    f.processes[1].gpus[0].usage.busy = None;
    h.send(Msg::Frame(Arc::new(f)));
    choose(&mut h, 2);
    assert_eq!(h.program().frame.groups[1].usage.graphics.busy, None);
    assert_eq!(h.program().rows[0], RowRef::Group(0));
    h.send(Msg::Sort(1, true));
    assert_eq!(h.program().rows[0], RowRef::Group(0));
    assert!(h.texts(h.window()).iter().any(|s| s == "—"));
}

#[test]
fn an_unavailable_driver_does_not_turn_missing_clients_into_zeroes() {
    let mut h = start();
    let mut f = frame(2);
    f.gpus[1].unlisted_usage = GpuUsage::UNKNOWN;
    for p in &mut f.processes {
        p.gpus.retain(|g| g.device != 1);
    }
    h.send(Msg::Frame(Arc::new(f)));
    choose(&mut h, 2);
    assert_eq!(h.program().frame.processes[1].usage.graphics.vram, None);
    assert_eq!(h.program().frame.groups[1].usage.graphics.busy, None);
    assert!(h.program().rows.is_empty());
    assert!(h.texts(h.window()).iter().any(|s| s == "GPU process data is unavailable."));
    choose(&mut h, 1);
    assert_eq!(h.program().frame.processes[0].usage.graphics.vram, Some(6 << 20));
    assert_eq!(h.program().frame.processes[1].usage.graphics.vram, Some(0));
}

#[test]
fn gpu_views_render_at_normal_and_minimum_sizes() {
    let mut h = start();
    let w = h.window();
    let dir = std::env::temp_dir().join("myaku-gpu");
    std::fs::create_dir_all(&dir).unwrap();
    h.save_png(w, dir.join("all.png"), 1.0);
    choose(&mut h, 2);
    h.save_png(w, dir.join("nvidia.png"), 1.0);
    h.send(Msg::Select(Sel::Group((20, 2))));
    h.send(Msg::Inspector(true));
    h.save_png(w, dir.join("inspector.png"), 1.0);
    h.send(Msg::Inspector(false));
    h.resize(w, Size::new(760.0, 480.0));
    h.save_png(w, dir.join("minimum.png"), 1.0);
    assert!(h.find_id(w, "gpu-selector").unwrap().width() <= 280.0);
    let amd = h.find_id(w, "gpu-device-0").unwrap();
    let nvidia = h.find_id(w, "gpu-device-1").unwrap();
    assert!(amd.x1 < nvidia.x0 && nvidia.x1 <= 760.0);
    assert!(h.texts(w).iter().any(|s| s == "22.0 MiB"));
    assert!(h.texts(w).iter().any(|s| s == "2.9 GiB"));

    let (signals, _) = Myaku::recorder();
    let mut day = Harness::with(
        Myaku::with_frame(Arc::new(frame(1)), signals, Request { lens: Some(Lens::Gpu), ..Default::default() }),
        Appearance { theme: ThemeChoice::Day, ..Appearance::default() },
        false,
    );
    let dw = day.window();
    choose(&mut day, 2);
    day.resize(dw, Size::new(760.0, 480.0));
    day.save_png(dw, dir.join("minimum-day.png"), 1.0);

    let mut many = frame(2);
    let mut third = many.gpus[1].clone();
    third.pci = "0000:02:00.0".into();
    many.gpus.push(third);
    h.send(Msg::Frame(Arc::new(many)));
    h.save_png(w, dir.join("three-gpus.png"), 1.0);
    choose(&mut h, 3);
    assert_eq!(h.program().gpu.as_deref(), Some("0000:02:00.0"));
    assert!(h.program().rows.is_empty());
    assert!(h.texts(w).iter().any(|s| s == "No processes are using this GPU."));

    let mut single = frame(3);
    single.gpus.truncate(1);
    h.send(Msg::Frame(Arc::new(single)));
    h.save_png(w, dir.join("one-gpu.png"), 1.0);
    assert!(h.find_id(w, "gpu-selector").is_none());
    assert!(h.find_id(w, "gpu-device-0").is_some());
    eprintln!("GPU renderings: {}", dir.display());
}

#[test]
#[ignore = "requires an NVIDIA GPU, driver and nvidia-smi"]
fn live_nvidia_memory_matches_nvidia_smi() {
    let mut sampler = myaku::sampler::Sampler::new();
    let _ = sampler.sample();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let f = sampler.sample();
    let gpu = f.gpus.iter().find(|g| g.driver == "nvidia").expect("NVIDIA GPU");
    assert!(gpu.vram_used.is_some_and(|n| n > 0));
    let output = std::process::Command::new("nvidia-smi")
        .args(["-i", &gpu.pci, "--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let mib: u64 = String::from_utf8(output.stdout).unwrap().trim().parse().unwrap();
    eprintln!("{}: Myaku {} MiB, nvidia-smi {mib} MiB", gpu.name, gpu.vram_used.unwrap() >> 20);
    assert!(gpu.vram_used.unwrap().abs_diff(mib << 20) < 128 << 20, "allow allocations between samples");
    let output = std::process::Command::new("nvidia-smi")
        .args(["-i", &gpu.pci, "--query-compute-apps=pid,used_gpu_memory", "--format=csv,noheader,nounits"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let mut matched = 0;
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        let Some((pid, memory)) = line.split_once(',') else { continue };
        let (Ok(pid), Ok(mib)) = (pid.trim().parse::<u32>(), memory.trim().parse::<u64>()) else { continue };
        let Some(p) = f.processes.iter().find(|p| p.pid == pid) else { continue };
        let device = f.gpus.iter().position(|g| g.pci == gpu.pci).unwrap();
        let usage = p.gpus.iter().find(|g| g.device == device).expect("NVIDIA process measurement");
        let bytes = usage.usage.vram.expect("process VRAM");
        eprintln!("{} ({pid}): Myaku {} MiB, nvidia-smi {mib} MiB", p.name, bytes >> 20);
        assert!(bytes.abs_diff(mib << 20) < 32 << 20);
        matched += 1;
    }
    assert!(matched > 0, "at least one compute process must be running");
}
