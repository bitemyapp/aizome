//! Shashin, run headlessly: opening a picture among its folder's, zoom,
//! pan, pinch, turning and saving, the Trash and Undo, the wallpaper,
//! Show in Hako, copying, the filmstrip, animations, SVGs, Get Info, full
//! screen and the slideshow; and golden images of its views.

use std::path::{Path, PathBuf};
use std::time::Duration;

use aizome_kit::prelude::*;
use aizome_kit::sidestep_ui::{Modifiers, NamedKey};
use aizome_kit::testing::{Harness, assert_golden};
use aizome_kit::theme::ThemeChoice;
use aizome_kit::{Appearance, WindowState};
use aizome_shashin::app::{Body, Wallpaper};
use aizome_shashin::view::Zoom;
use aizome_shashin::{Msg, Shashin};

// Fixtures.

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("shashin-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A landscape at dusk: sky in the dye ramp, a sun, two ridges, water.
fn landscape(w: u32, h: u32) -> image::RgbImage {
    let lerp = |a: [f32; 3], b: [f32; 3], t: f32| [0, 1, 2].map(|i| (a[i] + (b[i] - a[i]) * t.clamp(0.0, 1.0)) as u8);
    image::RgbImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let sun = ((fx - 0.68).powi(2) + (fy - 0.42).powi(2) * 0.6).sqrt();
        let ridge1 = 0.58 + 0.08 * (fx * 7.0).sin() + 0.04 * (fx * 17.0).cos();
        let ridge2 = 0.66 + 0.05 * (fx * 4.0 + 1.0).sin();
        let px = if fy > 0.8 {
            lerp([36.0, 40.0, 59.0], [22.0, 22.0, 30.0], (fy - 0.8) * 5.0)
        } else if fy > ridge2 {
            [36, 40, 59]
        } else if fy > ridge1 {
            [61, 89, 161]
        } else if sun < 0.07 {
            [224, 175, 104]
        } else {
            lerp([169.0, 196.0, 255.0], [122.0, 162.0, 247.0], fy * 1.6)
        };
        // A little grain, as a photo has.
        let n = ((x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 7) as u8;
        image::Rgb([px[0].saturating_add(n), px[1].saturating_add(n), px[2].saturating_add(n)])
    })
}

/// A camera's EXIF (big-endian TIFF): make, model, and settings in the
/// EXIF sub-IFD.
fn camera_exif() -> Vec<u8> {
    enum V {
        Ascii(&'static str),
        Short(u16),
        Long(u32),
        Rational(u32, u32),
    }
    fn ifd(out: &mut Vec<u8>, at: usize, entries: &[(u16, V)], data: &mut Vec<u8>, data_at: usize) {
        out.extend_from_slice(&(entries.len() as u16).to_be_bytes());
        for (tag, v) in entries {
            out.extend_from_slice(&tag.to_be_bytes());
            let (kind, count, inline): (u16, u32, Option<[u8; 4]>) = match v {
                V::Ascii(s) => (2, s.len() as u32 + 1, None),
                V::Short(n) => (3, 1, Some([(n >> 8) as u8, *n as u8, 0, 0])),
                V::Long(n) => (4, 1, Some(n.to_be_bytes())),
                V::Rational(..) => (5, 1, None),
            };
            out.extend_from_slice(&kind.to_be_bytes());
            out.extend_from_slice(&count.to_be_bytes());
            match inline {
                Some(b) => out.extend_from_slice(&b),
                None => {
                    out.extend_from_slice(&((data_at + data.len()) as u32).to_be_bytes());
                    match v {
                        V::Ascii(s) => {
                            data.extend_from_slice(s.as_bytes());
                            data.push(0);
                            if data.len() % 2 == 1 {
                                data.push(0);
                            }
                        }
                        V::Rational(a, b) => {
                            data.extend_from_slice(&a.to_be_bytes());
                            data.extend_from_slice(&b.to_be_bytes());
                        }
                        _ => {}
                    }
                }
            }
        }
        out.extend_from_slice(&0u32.to_be_bytes());
        let _ = at;
    }
    // IFD0 at 8 (3 entries: 42 bytes), its data after; then the EXIF IFD.
    let ifd0_len = 2 + 3 * 12 + 4;
    let ifd0_data = ["Fujifilm", "X100V"].iter().map(|s| (s.len() + 2) & !1).sum::<usize>();
    let exif_at = 8 + ifd0_len + ifd0_data;
    let mut t = b"MM\0\x2a\0\0\0\x08".to_vec();
    let mut data = Vec::new();
    ifd(&mut t, 8, &[(0x010f, V::Ascii("Fujifilm")), (0x0110, V::Ascii("X100V")), (0x8769, V::Long(exif_at as u32))], &mut data, 8 + ifd0_len);
    t.extend_from_slice(&data);
    assert_eq!(t.len(), exif_at);
    let exif_len = 2 + 5 * 12 + 4;
    let mut data = Vec::new();
    ifd(
        &mut t,
        exif_at,
        &[
            (0x829a, V::Rational(1, 250)),
            (0x829d, V::Rational(2, 1)),
            (0x8827, V::Short(200)),
            (0x9003, V::Ascii("2026:10:17 06:12:00")),
            (0x920a, V::Rational(23, 1)),
        ],
        &mut data,
        exif_at + exif_len,
    );
    t.extend_from_slice(&data);
    t
}

fn jpeg_with_exif(img: &image::RgbImage, exif: &[u8]) -> Vec<u8> {
    let mut plain = std::io::Cursor::new(Vec::new());
    img.write_to(&mut plain, image::ImageFormat::Jpeg).unwrap();
    let plain = plain.into_inner();
    let mut out = plain[..2].to_vec();
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&((exif.len() + 8) as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(exif);
    out.extend_from_slice(&plain[2..]);
    out
}

/// Pixel art with alpha: a 16 × 16 sprite.
fn sprite() -> image::RgbaImage {
    const ART: [&str; 16] = [
        "................",
        "......####......",
        "....##aaaa##....",
        "...#aaaaaaaa#...",
        "..#aaaaaaaaaa#..",
        "..#aa##aa##aa#..",
        ".#aaa##aa##aaa#.",
        ".#aaaaaaaaaaaa#.",
        ".#aaaaaaaaaaaa#.",
        ".#aa#aaaaaa#aa#.",
        "..#aa######aa#..",
        "..#aaaaaaaaaa#..",
        "...#aaaaaaaa#...",
        "....##aaaa##....",
        "......####......",
        "................",
    ];
    image::RgbaImage::from_fn(16, 16, |x, y| match ART[y as usize].as_bytes()[x as usize] {
        b'#' => image::Rgba([36, 40, 59, 255]),
        b'a' => image::Rgba([224, 175, 104, 255]),
        _ => image::Rgba([0, 0, 0, 0]),
    })
}

fn gif(frames: usize) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        enc.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
        let frames = (0..frames).map(|i| {
            let img = image::RgbaImage::from_fn(48, 32, |x, _| {
                if (x as usize / 8) % frames == i { image::Rgba([122, 162, 247, 255]) } else { image::Rgba([36, 40, 59, 255]) }
            });
            image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(200, 1))
        });
        enc.encode_frames(frames).unwrap();
    }
    out
}

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="240" viewBox="0 0 320 240">
<rect width="320" height="240" fill="#16161e"/>
<rect x="40" y="40" width="80" height="40" fill="#e0af68"/>
<rect x="40" y="80" width="160" height="40" fill="#a9c4ff"/>
<rect x="40" y="120" width="160" height="40" fill="#7aa2f7"/>
<rect x="40" y="160" width="160" height="40" fill="#4a67b3"/>
<circle cx="250" cy="120" r="44" fill="none" stroke="#e0af68" stroke-width="6"/>
</svg>"##;

/// A folder of pictures of every kind (and a text file), as Hako sorts
/// them: app.ico, beach.png, dance.gif, IMG_2.jpg, IMG_10.jpg, logo.svg,
/// old.bmp, scan.tiff, sticker.webp.
fn folder(name: &str) -> PathBuf {
    let d = scratch(name);
    std::fs::write(d.join("IMG_2.jpg"), jpeg_with_exif(&landscape(1600, 1066), &camera_exif())).unwrap();
    image::DynamicImage::ImageRgb8(landscape(800, 1200)).save(d.join("IMG_10.jpg")).unwrap();
    sprite().save(d.join("beach.png")).unwrap();
    std::fs::write(d.join("dance.gif"), gif(3)).unwrap();
    std::fs::write(d.join("logo.svg"), SVG).unwrap();
    image::DynamicImage::ImageRgb8(landscape(64, 48)).save(d.join("old.bmp")).unwrap();
    image::DynamicImage::ImageRgb8(landscape(96, 64)).save(d.join("scan.tiff")).unwrap();
    sprite().save(d.join("sticker.webp")).unwrap();
    image::imageops::resize(&sprite(), 32, 32, image::imageops::FilterType::Nearest).save(d.join("app.ico")).unwrap();
    std::fs::write(d.join("notes.txt"), "not a picture").unwrap();
    d
}

struct Rig {
    h: Harness<Shashin>,
    dir: PathBuf,
}

fn rig(name: &str, theme: ThemeChoice, open: &str) -> Rig {
    let dir = folder(name);
    let mut app = Shashin::new();
    app.trash = aizome_shashin::trash::Trash { home: dir.join(".trash-can"), uid: 1000 };
    app.settings_path = Some(dir.join("settings.toml"));
    app.thumb_dir = dir.join(".thumbnails");
    app.dry_run = true;
    if !open.is_empty() {
        app.initial = vec![dir.join(open)];
    }
    let h = Harness::with(app, Appearance { theme, ..Appearance::default() }, false);
    let mut r = Rig { h, dir };
    settle(&mut r, 1.0);
    r
}

/// Draw, take in what the background sent, and again, until nothing is
/// being read or drawn.
fn settle(r: &mut Rig, scale: f64) {
    let w = r.h.window();
    let mut quiet = 0;
    for _ in 0..1000 {
        let _ = r.h.render(w, scale);
        r.h.send(Msg::Nothing);
        if r.h.program().idle() {
            quiet += 1;
            if quiet >= 3 {
                return;
            }
        } else {
            quiet = 0;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    panic!("Shashin didn't settle");
}

fn names(r: &Rig) -> Vec<String> {
    r.h.program().folder.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
}

fn current(r: &Rig) -> String {
    r.h.program().current_path().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).unwrap_or_default()
}

fn key(r: &mut Rig, k: NamedKey) {
    let w = r.h.window();
    r.h.named(w, k, Modifiers::NONE);
}

// Behaviour.

#[test]
fn opens_a_picture_among_its_folder_in_hakos_order() {
    let mut r = rig("order", ThemeChoice::Night, "IMG_2.jpg");
    assert_eq!(
        names(&r),
        ["app.ico", "beach.png", "dance.gif", "IMG_2.jpg", "IMG_10.jpg", "logo.svg", "old.bmp", "scan.tiff", "sticker.webp"]
    );
    assert_eq!(current(&r), "IMG_2.jpg");
    let w = r.h.window();
    assert_eq!(r.h.title(w), "IMG_2.jpg");
    key(&mut r, NamedKey::ArrowRight);
    assert_eq!(current(&r), "IMG_10.jpg");
    key(&mut r, NamedKey::ArrowLeft);
    key(&mut r, NamedKey::ArrowLeft);
    assert_eq!(current(&r), "dance.gif");
    key(&mut r, NamedKey::End);
    assert_eq!(current(&r), "sticker.webp");
    // Round from the last to the first.
    key(&mut r, NamedKey::ArrowRight);
    assert_eq!(current(&r), "app.ico");
    // Every kind decodes.
    for _ in 0..9 {
        settle(&mut r, 1.0);
        let p = r.h.program().pic.as_ref().expect("a picture");
        assert!(p.info.error.is_none(), "{}: {:?}", current(&r), p.info.error);
        assert!(p.info.dims.0 > 0);
        key(&mut r, NamedKey::ArrowRight);
    }
}

#[test]
fn the_neighbours_are_read_ahead() {
    let mut r = rig("ahead", ThemeChoice::Night, "IMG_2.jpg");
    settle(&mut r, 1.0);
    let ahead: Vec<String> = r.h.program().ahead.iter().map(|p| p.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert!(ahead.contains(&"IMG_10.jpg".to_owned()) && ahead.contains(&"dance.gif".to_owned()), "{ahead:?}");
    key(&mut r, NamedKey::ArrowRight);
    // Shown at once, from what was read ahead.
    assert!(r.h.program().pic.as_ref().is_some_and(|p| p.decoded));
}

#[test]
fn fits_then_zooms_about_the_pointer_and_pans() {
    let mut r = rig("zoom", ThemeChoice::Night, "IMG_2.jpg");
    let w = r.h.window();
    // 1600 × 1066 in a 960 × 610 view: fitted.
    let fit = r.h.program().z();
    assert_eq!(r.h.program().zoom, Zoom::Fit);
    assert!((fit - 0.5722).abs() < 0.001, "{fit}");
    r.h.command(w, '0');
    assert_eq!(r.h.program().zoom, Zoom::Fixed(1.0));
    let c = r.h.program().center;
    assert!((c.0 - 800.0).abs() < 1.0 && (c.1 - 533.0).abs() < 1.0, "{c:?}");
    // Drag the picture left and up: the view moves right and down.
    r.h.drag(w, Point::new(500.0, 300.0), Point::new(400.0, 250.0), 4);
    let c2 = r.h.program().center;
    assert!(((c2.0 - c.0) - 100.0).abs() < 1e-9 && ((c2.1 - c.1) - 50.0).abs() < 1e-9, "{c:?} {c2:?}");
    // ❖-scroll zooms about the pointer, a quarter a notch.
    r.h.key(w, aizome_kit::sidestep_ui::Key::Character("=".into()), Modifiers::SUPER);
    assert_eq!(r.h.program().z(), 1.5);
    r.h.command(w, '-');
    assert_eq!(r.h.program().z(), 1.0);
    // A pinch.
    r.h.pinch(w, Point::new(300.0, 300.0), 2.0);
    assert!((r.h.program().z() - 2.0).abs() < 1e-9);
    // Double-click goes back to the fit.
    r.h.press(w, Point::new(300.0, 300.0), aizome_kit::sidestep_ui::PointerButton::Left, 2);
    r.h.release(w, Point::new(300.0, 300.0), aizome_kit::sidestep_ui::PointerButton::Left);
    assert_eq!(r.h.program().zoom, Zoom::Fit);
    r.h.command(w, '9');
    assert_eq!(r.h.program().zoom, Zoom::Fit);
}

#[test]
fn draws_a_picture_made_for_the_zoom_once_it_settles() {
    let mut r = rig("tiles", ThemeChoice::Night, "IMG_2.jpg");
    let w = r.h.window();
    for zoom in [None, Some(0.75), Some(1.0), Some(1.5), Some(3.0)] {
        match zoom {
            None => r.h.send(Msg::Fit),
            Some(z) => {
                r.h.send(Msg::ActualSize);
                if z != 1.0 {
                    r.h.pinch(w, Point::new(480.0, 300.0), z);
                }
            }
        }
        settle(&mut r, 2.0);
        let p = r.h.program();
        let z = p.z();
        let pic = p.pic.as_ref().unwrap();
        let exact = pic.tiles.iter().any(|t| aizome_shashin::app::same_zoom(t.z, z))
            || (z > 2.0 && pic.tiles.iter().any(|t| t.z == 1.0));
        assert!(exact, "no drawing for {z} among {:?}", pic.tiles.iter().map(|t| t.z).collect::<Vec<_>>());
        // At 2×, the scale the window draws at.
        assert_eq!(p.scale, 2.0);
    }
}

#[test]
fn turns_and_saves_without_reencoding_a_jpeg() {
    let mut r = rig("turn", ThemeChoice::Night, "IMG_2.jpg");
    let w = r.h.window();
    let path = r.dir.join("IMG_2.jpg");
    let before = std::fs::read(&path).unwrap();
    r.h.command(w, 'r');
    assert_eq!(r.h.program().shown_dims(), (1066.0, 1600.0));
    assert_eq!(r.h.title(w), "IMG_2.jpg — Turned");
    settle(&mut r, 1.0);
    // Turned drawings were made.
    assert!(r.h.program().pic.as_ref().unwrap().tiles.iter().any(|t| t.turns == 1));
    r.h.command(w, 's');
    settle(&mut r, 1.0);
    let after = std::fs::read(&path).unwrap();
    assert_eq!(aizome_shashin::orient::jpeg_orientation(&after), 6);
    // The compressed picture is the same bytes.
    assert!(after.ends_with(&before[before.len() - 4096..]));
    assert_eq!(r.h.title(w), "IMG_2.jpg");
    assert_eq!(r.h.program().shown_dims(), (1066.0, 1600.0), "upright by its tag now");
    assert!(r.h.program().note.as_ref().is_some_and(|n| n.text.contains("without re-encoding")));
    // A PNG is written again, turned.
    r.h.send(Msg::Go(1));
    settle(&mut r, 1.0);
    r.h.command(w, 'l');
    r.h.command(w, 's');
    settle(&mut r, 1.0);
    let png = image::open(r.dir.join("beach.png")).unwrap().to_rgba8();
    assert_eq!(png, image::imageops::rotate270(&sprite()));
}

#[test]
fn unsaved_turns_ask_before_quitting() {
    let mut r = rig("ask", ThemeChoice::Night, "beach.png");
    let w = r.h.window();
    r.h.command(w, 'r');
    r.h.command(w, 'q');
    assert!(r.h.texts(w).iter().any(|t| t == "Save a turned picture?"), "{:?}", r.h.texts(w));
    r.h.click_text(w, "Don't Save");
    assert!(r.h.platform.quit);
}

#[test]
fn moves_to_the_trash_and_back() {
    let mut r = rig("trash", ThemeChoice::Night, "IMG_2.jpg");
    let w = r.h.window();
    let path = r.dir.join("IMG_2.jpg");
    r.h.key(w, aizome_kit::sidestep_ui::Key::Named(NamedKey::Backspace), Modifiers::SUPER);
    assert!(!path.exists());
    assert!(r.dir.join(".trash-can/files/IMG_2.jpg").exists());
    assert_eq!(current(&r), "IMG_10.jpg");
    assert_eq!(r.h.program().folder.len(), 8);
    assert!(r.h.program().note.as_ref().is_some_and(|n| n.text.contains("Trash")));
    r.h.command(w, 'z');
    assert!(path.exists());
    assert_eq!(current(&r), "IMG_2.jpg");
    assert_eq!(r.h.program().folder.len(), 9);
    // And again.
    r.h.key(w, aizome_kit::sidestep_ui::Key::Character("z".into()), Modifiers::SUPER | Modifiers::SHIFT);
    assert!(!path.exists());
}

#[test]
fn sets_the_wallpaper_shows_in_hako_and_copies() {
    let mut r = rig("elsewhere", ThemeChoice::Night, "IMG_10.jpg");
    let w = r.h.window();
    r.h.send(Msg::SetWallpaper(Wallpaper::Night));
    settle(&mut r, 1.0);
    let s = aizome_config::Settings::load_from(&r.dir.join("settings.toml")).unwrap();
    assert_eq!(s.appearance.wallpaper_night.as_deref(), Some(r.dir.join("IMG_10.jpg").to_str().unwrap()));
    assert_eq!(s.appearance.wallpaper_day, None);
    r.h.send(Msg::ShowInHako);
    assert_eq!(r.h.program().commands, [vec!["aizome-hako".to_owned(), "--select".into(), r.dir.join("IMG_10.jpg").to_string_lossy().into_owned()]]);
    r.h.command(w, 'c');
    settle(&mut r, 1.0);
    assert!(r.h.program().copied.is_some_and(|n| n > 1000));
}

#[test]
fn the_filmstrip_shows_thumbnails_kept_in_the_freedesktop_cache() {
    let mut r = rig("strip", ThemeChoice::Night, "IMG_2.jpg");
    let w = r.h.window();
    r.h.key(w, aizome_kit::sidestep_ui::Key::Character("f".into()), Modifiers::NONE);
    assert!(r.h.program().filmstrip);
    settle(&mut r, 1.0);
    let ready = r.h.program().thumbs.values().filter(|t| matches!(t, aizome_shashin::app::Thumb::Ready(_))).count();
    assert_eq!(ready, 9);
    let cached = aizome_shashin::thumbs::thumb_path(&r.dir.join(".thumbnails"), &r.dir.join("IMG_2.jpg"));
    assert!(cached.exists());
    // Clicking a thumbnail goes to it: the slot right of the current one.
    r.h.click(w, Point::new(480.0 + 72.0, 640.0 - 38.0));
    assert_eq!(current(&r), "IMG_10.jpg");
}

#[test]
fn animations_play() {
    let mut r = rig("gif", ThemeChoice::Night, "dance.gif");
    let p = r.h.program().pic.as_ref().unwrap();
    assert!(matches!(p.info.body, Some(Body::Frames(..))));
    assert_eq!(p.frame, 0);
    r.h.advance(Duration::from_millis(210));
    assert_eq!(r.h.program().pic.as_ref().unwrap().frame, 1);
    r.h.advance(Duration::from_millis(400));
    assert_eq!(r.h.program().pic.as_ref().unwrap().frame, 0);
}

#[test]
fn svgs_are_drawn_at_the_zoom() {
    let mut r = rig("svg", ThemeChoice::Night, "logo.svg");
    let w = r.h.window();
    assert_eq!(r.h.program().pic.as_ref().unwrap().info.dims, (320, 240));
    for _ in 0..4 {
        r.h.key(w, aizome_kit::sidestep_ui::Key::Character("+".into()), Modifiers::SUPER | Modifiers::SHIFT);
    }
    assert_eq!(r.h.program().z(), 4.0);
    settle(&mut r, 1.0);
    let pic = r.h.program().pic.as_ref().unwrap();
    assert!(pic.tiles.iter().any(|t| t.z == 4.0), "{:?}", pic.tiles.iter().map(|t| t.z).collect::<Vec<_>>());
}

#[test]
fn get_info_tells_the_camera() {
    let mut r = rig("info", ThemeChoice::Night, "IMG_2.jpg");
    let w = r.h.window();
    r.h.command(w, 'i');
    let texts = r.h.texts(w);
    for want in ["JPEG picture", "1600 × 1066 pixels", "Fujifilm X100V", "camera"] {
        assert!(texts.iter().any(|t| t.to_lowercase().contains(&want.to_lowercase())), "{want} not in {texts:?}");
    }
    let settings = texts.iter().find(|t| t.contains("ISO")).expect("settings");
    assert!(settings.contains("23 mm") && settings.contains("ƒ/2") && settings.contains("1/250 s") && settings.contains("ISO 200"), "{settings}");
    assert!(texts.iter().any(|t| t.contains("2026-10-17 06:12:00") || t.contains("2026:10:17")), "{texts:?}");
    r.h.named(w, NamedKey::Enter, Modifiers::NONE);
    assert!(r.h.program().info_sheet.is_none());
}

#[test]
fn what_isnt_a_picture_says_so() {
    let dir = folder("bad");
    std::fs::write(dir.join("broken.png"), b"\x89PNG\r\n\x1a\nnot really").unwrap();
    let mut app = Shashin::new();
    app.initial = vec![dir.join("broken.png")];
    app.dry_run = true;
    app.thumb_dir = dir.join(".thumbnails");
    let h = Harness::with(app, Appearance::default(), false);
    let mut r = Rig { h, dir };
    settle(&mut r, 1.0);
    let w = r.h.window();
    assert!(r.h.texts(w).iter().any(|t| t.contains("Shashin can't open “broken.png”")), "{:?}", r.h.texts(w));
}

#[test]
fn full_screen_and_the_slideshow() {
    let mut r = rig("show", ThemeChoice::Night, "IMG_2.jpg");
    let w = r.h.window();
    r.h.key(w, aizome_kit::sidestep_ui::Key::Character("F".into()), Modifiers::SUPER | Modifiers::SHIFT);
    assert_eq!(r.h.fullscreen_asked(w), Some(true));
    r.h.window_state(w, WindowState { fullscreen: true, maximized: false });
    assert!(r.h.program().fullscreen);
    // No toolbar in full screen.
    assert!(!r.h.texts(w).iter().any(|t| t.contains(" of 9")));
    r.h.advance(Duration::from_secs(5));
    assert_eq!(current(&r), "IMG_10.jpg");
    r.h.advance(Duration::from_secs(5));
    assert_eq!(current(&r), "logo.svg");
    key(&mut r, NamedKey::Escape);
    assert!(r.h.program().slideshow.is_none());
    assert_eq!(r.h.fullscreen_asked(w), Some(false));
    r.h.window_state(w, WindowState::default());
    assert!(!r.h.program().fullscreen);
    // The desktop's ⌃❖F, heard as the window's state.
    r.h.window_state(w, WindowState { fullscreen: true, maximized: false });
    assert!(r.h.program().fullscreen);
}

#[test]
fn a_second_launch_hands_its_files_over() {
    let dir = scratch("instance");
    let path = dir.join("img.png");
    sprite().save(&path).unwrap();
    // A socket of our own, not the session's.
    let name = format!("shashin-test-{}", std::process::id());
    let aizome_shashin::instance::Claim::First(listener) = aizome_shashin::instance::claim(&name, &[]) else { panic!("first") };
    let mut app = Shashin::new();
    app.instance = Some(listener);
    app.dry_run = true;
    app.thumb_dir = dir.join(".thumbnails");
    let h = Harness::with(app, Appearance::default(), false);
    let mut r = Rig { h, dir: dir.clone() };
    assert!(matches!(aizome_shashin::instance::claim(&name, std::slice::from_ref(&path)), aizome_shashin::instance::Claim::Handed));
    for _ in 0..200 {
        r.h.send(Msg::Nothing);
        if r.h.program().current_path() == Some(&path) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(r.h.program().current_path(), Some(&path));
    let _ = std::fs::remove_file(aizome_shashin::instance::socket_path(&name));
}

// Pictures.

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.png"))
}

fn pictures(r: &mut Rig, name: &str) {
    let w = r.h.window();
    settle(r, 2.0);
    r.h.save_png(w, Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("shashin-{name}@2x.png")), 2.0);
    if !r.h.theme().has_ui_font() {
        eprintln!("skipping the golden {name}: Archivo isn't installed (set AIZOME_FONT_DIR=~/work/.aizome-fonts)");
        return;
    }
    settle(r, 1.0);
    assert_golden(&mut r.h, w, golden(name), 1.0);
}

#[test]
fn goldens() {
    for (theme, t) in [(ThemeChoice::Night, "night"), (ThemeChoice::Day, "day")] {
        let mut r = rig(&format!("golden-{t}"), theme, "IMG_2.jpg");
        pictures(&mut r, &format!("photo-{t}"));
        let w = r.h.window();
        r.h.key(w, aizome_kit::sidestep_ui::Key::Character("f".into()), Modifiers::NONE);
        pictures(&mut r, &format!("filmstrip-{t}"));
        r.h.command(w, 'i');
        pictures(&mut r, &format!("info-{t}"));
    }
    let mut r = rig("golden-pixels", ThemeChoice::Night, "beach.png");
    let w = r.h.window();
    r.h.send(Msg::ZoomPick(6));
    pictures(&mut r, "pixels-800-night");
    r.h.send(Msg::Go(5));
    pictures(&mut r, "svg-night");
    r.h.command(w, 'r');
    pictures(&mut r, "svg-turned-night");

    let mut app = Shashin::new();
    app.dry_run = true;
    let h = Harness::with(app, Appearance { theme: ThemeChoice::Day, ..Appearance::default() }, false);
    let mut r = Rig { h, dir: PathBuf::new() };
    pictures(&mut r, "empty-day");
}
