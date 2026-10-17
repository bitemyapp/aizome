//! The window, headless: pages, the sunburst's clicks and drags, the
//! Collector and the Trash, and golden images of the main views.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use aizome_kit::sidestep_ui::PointerButton;
use aizome_kit::testing::{Harness, assert_golden};
use aizome_kit::theme::{Appearance, ThemeChoice};
use aizome_kit::{Point, Size};
use kura::mounts::Mount;
use kura::nix::StoreSize;
use kura::scan::{ProgressSnapshot, Scan, ScanOptions};
use kura::tree::{Entry, Kind, NodeId, Tree};
use kura::ui::rings::{self, Geometry, RINGS};
use kura::ui::{Kura, Msg, Page};

const GB: u64 = 1_000_000_000;
const MB: u64 = 1_000_000;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kura-ui-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

/// A made-up home: what a developer's often holds.
enum Fake {
    F(&'static str, u64),
    D(&'static str, Vec<Fake>),
}
use Fake::{D, F};

fn many(prefix: &'static str, n: usize, each: u64) -> Vec<Fake> {
    (0..n).map(|i| F(Box::leak(format!("{prefix}-{i:03}.jpg").into_boxed_str()), each + (i as u64 * 7919) % (each / 3 + 1))).collect()
}

fn home() -> Fake {
    D("/home/sekka", vec![
        D("work", vec![
            D("aizome", vec![
                D("target", vec![D("debug", vec![
                    D("deps", vec![F("libsidestep_ui.rlib", 9 * GB), F("libsmithay.rlib", 6 * GB), F("aizome-comp", 4 * GB)]),
                    D("build", vec![F("out.o", 5 * GB)]),
                    D("incremental", vec![F("state.bin", 3 * GB)]),
                ])]),
                D("crates", vec![F("lib.rs", 2 * MB), F("main.rs", MB)]),
                F("Cargo.toml", 4096),
            ]),
            D("sekka", vec![D("result", vec![F("system.img", 3 * GB)]), D("desktop", vec![F("wallpapers.tar", 2 * GB)]), F("flake.lock", 8192)]),
            D("notes", vec![F("plan.md", 40_000)]),
        ]),
        D("Videos", vec![F("holiday-2025.mkv", 8 * GB), F("conference-talk.mp4", 3 * GB), D("clips", vec![F("a.mp4", GB), F("b.mp4", 700 * MB)])]),
        D("Pictures", vec![D("2025", many("IMG", 60, 90 * MB)), D("wallpapers", many("wall", 12, 40 * MB))]),
        D("Downloads", vec![F("sekka-nightly.iso", 3 * GB + 400 * MB), F("old-backup.zip", 1200 * MB), F("installer.AppImage", 400 * MB)]),
        D(".cache", vec![D("google-chrome", vec![F("data_1", 1900 * MB)]), D("pip", vec![F("wheels", 800 * MB)]), D("nix", vec![F("eval.sqlite", 600 * MB)])]),
        D(".local", vec![D("share", vec![D("containers", vec![F("layers.tar", 4 * GB)]), D("Trash", vec![F("old-vm.qcow2", 1100 * MB)])])]),
        D("Documents", vec![F("thesis.pdf", 30 * MB), F("taxes-2025.pdf", 4 * MB), D("scans", many("scan", 20, 15 * MB))]),
        D("Music", vec![D("albums", many("track", 40, 60 * MB))]),
        F(".bash_history", 200_000),
    ])
}

fn push(t: &mut Tree, at: NodeId, kids: &[Fake]) {
    let entries: Vec<Entry> = kids
        .iter()
        .map(|k| match k {
            F(n, s) => Entry { name: n.as_bytes().to_vec(), kind: Kind::File, flags: 0, alloc: *s, apparent: *s, mtime: 1_760_000_000 },
            D(n, _) => Entry { name: n.as_bytes().to_vec(), kind: Kind::Dir, flags: 0, alloc: 4096, apparent: 4096, mtime: 1_760_000_000 },
        })
        .collect();
    let first = t.push_children(at, &entries);
    for (i, k) in kids.iter().enumerate() {
        if let D(_, sub) = k
            && !sub.is_empty()
        {
            push(t, NodeId(first + i as u32), sub);
        }
    }
}

fn fake_scan() -> Arc<Scan> {
    let D(root, kids) = home() else { unreachable!() };
    let mut t = Tree::new(PathBuf::from(root), Kind::Dir, 4096, 4096, 1_760_000_000);
    push(&mut t, NodeId::ROOT, &kids);
    t.finish();
    Arc::new(Scan {
        tree: t,
        options: ScanOptions::new(root),
        started: 1_760_000_000,
        duration: Duration::from_millis(840),
        error_count: 0,
        errors: Vec::new(),
        other_filesystems: Vec::new(),
        hard_links: 0,
        threads: 8,
    })
}

fn fake_mounts() -> Vec<Mount> {
    let m = |p: &str, s: &str, t: &str, total: u64, free: u64| Mount {
        mount_point: PathBuf::from(p),
        source: s.into(),
        fs_type: t.into(),
        total_bytes: total,
        free_bytes: free,
        available_bytes: free,
        read_only: false,
    };
    vec![
        m("/", "/dev/nvme0n1p2", "ext4", 469 * GB, 17 * GB),
        m("/boot", "/dev/nvme0n1p1", "vfat", GB, 800 * MB),
        m("/run/media/sekka/Archive", "/dev/sda1", "btrfs", 2000 * GB, 1240 * GB),
    ]
}

/// Run the program's background work until `done` says so.
fn settle(h: &mut Harness<Kura>, done: impl Fn(&Kura) -> bool) {
    for _ in 0..600 {
        if done(h.program()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
        h.advance(Duration::from_millis(100));
        h.send(Msg::Poll);
    }
    panic!("the program didn't settle");
}

/// Kura in `look`, writing only into `out`, with fixed disks and store.
fn kura(out: &Path, look: Appearance) -> Harness<Kura> {
    let mut k = Kura::sandboxed(out);
    k.has_store = true;
    k.index_dir = None;
    let mut h = Harness::with(k, look, false);
    // The real disks arrive first; then the made-up ones replace them.
    settle(&mut h, |k| !k.mounts.is_empty());
    h.send(Msg::Disks(fake_mounts()));
    h.send(Msg::StoreMeasured(Some(StoreSize { bytes: 54_400_000_000, paths: 37_879, ..StoreSize::default() })));
    let w = h.window();
    h.resize(w, Size::new(960.0, 640.0));
    h
}

fn night() -> Appearance {
    Appearance::default()
}

fn day() -> Appearance {
    Appearance { theme: ThemeChoice::Day, ..Appearance::default() }
}

fn with_results(h: &mut Harness<Kura>) {
    h.send(Msg::Scanned(Ok(fake_scan())));
    h.send(Msg::Indexed(Ok((PathBuf::from("/home/sekka/.cache/aizome/kura/index.json"), 31 * GB))));
    assert_eq!(h.program().page, Page::Results);
}

/// The middle of segment `name` (a child of the centre) in window points.
fn point_on(h: &Harness<Kura>, name: &str) -> Point {
    let w = h.window();
    let rect = h.find_id(w, "sunburst").expect("the sunburst");
    let k = h.program();
    let tree = k.tree().unwrap();
    let segs = rings::segments(tree, k.center, RINGS);
    let seg = segs.iter().find(|s| s.depth == 1 && s.node.is_some_and(|n| tree.name(n) == name)).expect("its segment");
    let g = Geometry::new(rect.size(), RINGS);
    let a = Geometry::angle((seg.x0 + seg.x1) / 2.0);
    let r = g.hole + g.ring * 0.5;
    Point::new(rect.x0 + g.center.x + r * a.cos(), rect.y0 + g.center.y + r * a.sin())
}

fn center_point(h: &Harness<Kura>) -> Point {
    let rect = h.find_id(h.window(), "sunburst").unwrap();
    let g = Geometry::new(rect.size(), RINGS);
    Point::new(rect.x0 + g.center.x, rect.y0 + g.center.y)
}

#[test]
fn disks_page() {
    let out = scratch("disks");
    let mut h = kura(&out, night());
    let w = h.window();
    let texts = h.texts(w);
    assert!(texts.iter().any(|t| t == "System"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Archive"));
    assert!(texts.iter().any(|t| t == "Nix store"));
    assert!(texts.iter().any(|t| t.contains("54.4 GB in 37,879 store paths")));
    assert!(texts.iter().any(|t| t == "Open Yukimi's Storage"));
    assert_golden(&mut h, w, golden("disks-night.png"), 2.0);
    h.set_appearance(day());
    assert_golden(&mut h, w, golden("disks-day.png"), 2.0);
}

#[test]
fn scanning_page() {
    let out = scratch("scanning");
    let mut h = kura(&out, night());
    let w = h.window();
    h.program_mut().show_progress(
        PathBuf::from("/home/sekka"),
        ProgressSnapshot { files: 412_331, dirs: 38_212, bytes: 182_400_000_000, errors: 0, current: PathBuf::from("/home/sekka/work/aizome/target/debug/deps") },
    );
    h.send(Msg::Dismiss);
    assert!(h.texts(w).iter().any(|t| t == "412,331 files · 38,212 folders"));
    assert_golden(&mut h, w, golden("scanning-night.png"), 2.0);
}

#[test]
fn results_draw_and_drill() {
    let out = scratch("results");
    let mut h = kura(&out, night());
    let w = h.window();
    with_results(&mut h);
    let texts = h.texts(w);
    assert!(texts.iter().any(|t| t == "work"), "the list shows the top level: {texts:?}");
    assert_golden(&mut h, w, golden("results-night.png"), 2.0);
    h.set_appearance(day());
    assert_golden(&mut h, w, golden("results-day.png"), 2.0);
    h.set_appearance(night());

    // A click on a folder's segment goes into it; the middle goes back up.
    let p = point_on(&h, "work");
    h.click(w, p);
    let k = h.program();
    assert_eq!(k.tree().unwrap().name(k.center), "work");
    assert!(k.transition.is_some_and(|t| t.zoom_in), "going down a level moves the rings");
    h.click(w, center_point(&h));
    let k = h.program();
    assert_eq!(k.center, NodeId::ROOT);
    assert!(k.transition.is_some_and(|t| !t.zoom_in));
    assert_eq!(k.selected.map(|s| k.tree().unwrap().name(s).into_owned()).as_deref(), Some("work"));
}

#[test]
fn reduced_motion_cuts() {
    let out = scratch("reduced");
    let mut h = kura(&out, Appearance { reduced_motion: true, ..night() });
    with_results(&mut h);
    let w = h.window();
    let p = point_on(&h, "Videos");
    h.click(w, p);
    let k = h.program();
    assert_eq!(k.tree().unwrap().name(k.center), "Videos");
    assert!(k.transition.is_none());
}

#[test]
fn hovering_and_collecting() {
    let out = scratch("collect");
    let mut h = kura(&out, night());
    with_results(&mut h);
    let w = h.window();
    let tree = h.program().tree().unwrap().clone();
    let id = |p: &str| tree.find(Path::new(p)).unwrap();

    h.send(Msg::Collect(id("/home/sekka/work/sekka/result")));
    assert_eq!(h.program().collector.len(), 1);
    // Something inside what's collected is already collected; collecting
    // its folder takes the inner one's place.
    h.send(Msg::Collect(id("/home/sekka/work/sekka/result/system.img")));
    assert_eq!(h.program().collector.len(), 1);
    h.send(Msg::Collect(id("/home/sekka/work/sekka")));
    assert_eq!(h.program().collector, vec![id("/home/sekka/work/sekka")]);
    h.send(Msg::Collect(id("/home/sekka/Downloads/sekka-nightly.iso")));
    assert_eq!(h.program().collected_bytes(), tree.node(id("/home/sekka/work/sekka")).alloc + 3_400_000_000);

    h.move_to(w, point_on(&h, "Pictures"));
    let texts = h.texts(w);
    assert!(texts.iter().any(|t| t == "Move to Trash…"));
    assert_golden(&mut h, w, golden("results-hover-collector-night.png"), 2.0);
}

#[test]
fn dragging_a_segment_into_the_collector() {
    let out = scratch("drag");
    let mut h = kura(&out, night());
    with_results(&mut h);
    let w = h.window();
    // The Collector says where it is when it's drawn.
    let _ = h.render(w, 1.0);
    let zone = h.find_id(w, "collector").expect("the collector");
    let from = point_on(&h, "Videos");
    h.press(w, from, PointerButton::Left, 1);
    h.move_to(w, Point::new(from.x + 10.0, from.y + 10.0));
    assert!(h.program().dragging.is_some(), "moving past the slop starts a drag");
    let to = zone.center();
    h.move_to(w, to);
    assert!(h.program().drag_over);
    assert_golden(&mut h, w, golden("results-drag-night.png"), 2.0);
    h.release(w, to, PointerButton::Left);
    let k = h.program();
    assert!(k.dragging.is_none());
    assert_eq!(k.collector.len(), 1);
    assert_eq!(k.tree().unwrap().name(k.collector[0]), "Videos");
    assert_eq!(k.center, NodeId::ROOT, "a drag isn't a click");
}

#[test]
fn moving_to_the_trash_and_writing_the_index() {
    let out = scratch("trash");
    // A real folder this time: the scanner, the Trash and the index.
    let home = out.join("home");
    std::fs::create_dir_all(home.join("big")).unwrap();
    std::fs::write(home.join("big/blob.bin"), vec![7u8; 3_000_000]).unwrap();
    std::fs::write(home.join("keep.txt"), b"keep").unwrap();
    let mut k = Kura::sandboxed(&out);
    k.has_store = false;
    k.duplicates = false;
    let mut h = Harness::new(k);
    h.send(Msg::Scan(home.clone()));
    settle(&mut h, |k| k.page == Page::Results && matches!(k.index, kura::ui::IndexState::Written { .. }));
    let index = out.join("index/index.json");
    assert!(index.exists() && out.join("index/index.md").exists());
    let before = h.program().tree().unwrap().node(NodeId::ROOT).alloc;

    let big = h.program().tree().unwrap().find(&home.join("big")).unwrap();
    h.send(Msg::Collect(big));
    h.send(Msg::Trash);
    settle(&mut h, |k| k.collector.is_empty() && matches!(k.index, kura::ui::IndexState::Written { .. }));
    assert!(!home.join("big").exists());
    assert!(out.join("Trash/files/big/blob.bin").exists());
    assert!(out.join("Trash/info/big.trashinfo").exists());
    let k = h.program();
    let after = k.tree().unwrap().node(NodeId::ROOT).alloc;
    assert!(before - after >= 3_000_000, "the tree no longer counts it");
    assert!(k.notice.as_deref().is_some_and(|n| n.starts_with("Moved 1 item to the Trash")));
    assert!(!k.rows.contains(&big));
    let ix = kura::index::read(&index).unwrap();
    assert!(ix.totals.allocated_bytes < before);
    assert!(!ix.largest_files.iter().any(|f| f.path.ends_with("blob.bin")));
}

#[test]
fn the_nix_store_is_never_collected() {
    let out = scratch("store");
    let mut h = kura(&out, night());
    let e = |n: &str, k: Kind, a: u64| Entry { name: n.as_bytes().to_vec(), kind: k, flags: 0, alloc: a, apparent: a, mtime: 0 };
    let mut t = Tree::new(PathBuf::from("/nix"), Kind::Dir, 4096, 4096, 0);
    let s = t.push_children(NodeId::ROOT, &[e("store", Kind::Dir, 4096), e("var", Kind::Dir, 4096)]);
    t.push_children(NodeId(s), &[e("abc123-firefox-140", Kind::Dir, 300 * MB)]);
    t.finish();
    let scan = Scan {
        tree: t,
        options: ScanOptions::new("/nix"),
        started: 0,
        duration: Duration::ZERO,
        error_count: 0,
        errors: Vec::new(),
        other_filesystems: Vec::new(),
        hard_links: 0,
        threads: 1,
    };
    h.send(Msg::Scanned(Ok(Arc::new(scan))));
    let tree = h.program().tree().unwrap().clone();
    for p in ["/nix/store", "/nix/store/abc123-firefox-140"] {
        h.send(Msg::Collect(tree.find(Path::new(p)).unwrap()));
        assert!(h.program().collector.is_empty(), "{p} isn't collected");
        assert!(h.program().notice.as_deref().is_some_and(|n| n.contains("Yukimi")));
    }
    // Inside the store, the list says how it's cleaned.
    h.send(Msg::Open(tree.find(Path::new("/nix/store")).unwrap()));
    assert!(h.texts(h.window()).iter().any(|t| t == "Open Yukimi's Storage"));
}
