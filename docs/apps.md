# Aizome's apps

Every app is a crate in `apps/<name>` built with `aizome-kit`. Names are
Japanese words that say what the app is for, in keeping with Aizome and Sekka
(Tatami, Yukimi, Genkan); each carries a plain English generic name, which
the Toolchest, the palette and the Deskbar show beside it and search by.

| App | Generic name | Crate | Binary | App ID | What the name means |
|---|---|---|---|---|---|
| Shitsurae しつらえ | Settings | `apps/shitsurae` | `aizome-shitsurae` | `org.aizome.Shitsurae` | arranging a room for its season and guests |
| Yukimi 雪見 | Software | `apps/yukimi` | `aizome-yukimi` | `org.aizome.Yukimi` | snow-viewing, through a shōji's low pane of glass (Sekka's Nix software manager, made native) |
| Hako 箱 | Files | `apps/hako` | `aizome-hako` | `org.aizome.Hako` | box |
| Sumi 墨 | Text Editor | `apps/sumi` | `aizome-sumi` | `org.aizome.Sumi` | ink |
| Fusen 付箋 | Notes | `apps/fusen` | `aizome-fusen` | `org.aizome.Fusen` | sticky notes |
| Koyomi 暦 | Calendar | `apps/koyomi` | `aizome-koyomi` | `org.aizome.Koyomi` | almanac, calendar |
| Tokei 時計 | Clock | `apps/tokei` | `aizome-tokei` | `org.aizome.Tokei` | clock |
| Soroban 算盤 | Calculator | `apps/soroban` | `aizome-soroban` | `org.aizome.Soroban` | abacus |
| Tenki 天気 | Weather | `apps/tenki` | `aizome-tenki` | `org.aizome.Tenki` | weather |
| Tansu 箪笥 | Disks | `apps/tansu` | `aizome-tansu` | `org.aizome.Tansu` | chest of drawers (partitions as drawers) |
| Kura 蔵 | Disk Usage | `apps/kura` | `aizome-kura` | `org.aizome.Kura` | storehouse |
| Myaku 脈 | Activity | `apps/myaku` | `aizome-myaku` | `org.aizome.Myaku` | pulse |

Also part of the session, not separate apps: the Toolchest, the command
palette, notifications, the lock screen and the polkit dialog (all in
`aizome-shell`). The terminal is foot, themed by Aizome's module.

## Conventions

- **Desktop entry** `org.aizome.<Name>.desktop`: `Name=<Name>`,
  `GenericName=<generic>`, `Comment`, `Keywords` (the generic name and
  common synonyms: Settings gets "preferences;control panel;system";
  Soroban gets "calculator;math;arithmetic"), `Categories`,
  `Icon=org.aizome.<Name>`, `Exec=aizome-<name> %F` (or `%U`),
  `StartupWMClass=org.aizome.<Name>`, `DBusActivatable=false`.
- **Icon**: `icons/apps/org.aizome.<Name>.svg` (drawn by `icons/genicons.py`),
  installed to `share/icons/hicolor/scalable/apps/`.
- **App ID** on every window (`xdg_toplevel.set_app_id`): the app ID above.
  The compositor treats `org.aizome.*` windows as speaking Aizome's command
  key natively.
- **One instance**: a second launch hands its arguments to the running one
  (a Unix socket in `$XDG_RUNTIME_DIR/aizome-app.<name>.sock`) and exits.
- **State** in `$XDG_STATE_HOME/aizome/<name>/`, **data** in
  `$XDG_DATA_HOME/aizome/<name>/`, **settings** in
  `$XDG_CONFIG_HOME/aizome/<name>.toml`; desktop-wide settings come from
  `aizome-config`.
- **Menus**: the kit's standard app, Edit, Window and Help menus, extended
  by the app; every command reachable from the palette.
- **Look**: only the kit's widgets and the theme's colours; Night and Day
  must both look finished; every target 24pt or more; all of it reachable
  by keyboard.
- **Frame budget**: 4 ms for a typical interaction; measure with
  `SIDESTEP_FRAME_STATS=1` and note exceptions in `docs/performance.md`.
- **Headless test**: each app has at least one test that runs it with
  `SIDESTEP_BACKEND=null`, drives it through messages and checks its state,
  and golden images of its main views through the offscreen renderer.
