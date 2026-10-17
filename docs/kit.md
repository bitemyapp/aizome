# aizome-kit

The widget kit every Aizome app and the shell are written with. It sits on
Sidestep's native toolkit (`sidestep-ui`), which gives windows, input, a
canvas and text layout, and adds everything an app needs on top: layout,
controls, lists, text editing, menus (published to the global menu bar),
sheets and panels, all in Aizome's look.

This is the guide for app authors. The rustdoc of each item has the
details.

## The model

An app is a [`Program`]: state, an `update` that changes it with messages,
and a `view` of each window, Elm's way.

```rust
use aizome_kit::prelude::*;
use aizome_kit::column; // std has a `column!` macro too: name the kit's

#[derive(Default)]
struct Soroban { value: f64 }

#[derive(Clone, Debug)]
enum Msg { Digit(u8), Clear }

impl Program for Soroban {
    type Msg = Msg;

    fn info(&self) -> AppInfo {
        AppInfo::new("org.aizome.Soroban", "Soroban", "Calculator").icon(Icon::Calculator)
    }

    fn launched(&mut self, cx: &mut Cx<Msg>) {
        cx.open_window(WindowSpec::new("Soroban").size(320.0, 480.0).resizable(false));
    }

    fn update(&mut self, msg: Msg, _cx: &mut Cx<Msg>) {
        match msg {
            Msg::Digit(d) => self.value = self.value * 10.0 + f64::from(d),
            Msg::Clear => self.value = 0.0,
        }
    }

    fn view(&self, _window: WindowKey) -> Element<Msg> {
        column![
            text(format!("{}", self.value)).size(28.0).mono().tabular(),
            row![button("7").on_press(Msg::Digit(7)), button("C").on_press(Msg::Clear)].spacing(8.0),
        ]
        .spacing(12.0)
        .padding(16.0)
        .into()
    }
}

fn main() {
    aizome_kit::run(Soroban::default())
}
```

- **Messages** are `Clone + Send`. Widgets send them (`on_press`,
  `on_change`); timers, background work and other threads send them through
  `Cx`. `update` gets one at a time; after a batch the kit views every
  window again.
- **Views are cheap to make and are made often.** An element is a
  description; the kit mounts it, lays it out, and diffs the result against
  the window's last tree by **key path**: each node's identity is its
  ancestors' keys and its own (`.key(k)`, else its place and kind among its
  siblings). A node whose rectangle and drawing hash are the same as last
  time isn't redrawn; the window's damage is the union of what changed.
- **Widget state** the program needn't keep — a caret, a selection, a scroll
  offset, what's hovered or pressed, a menu that's open — lives in the
  window's state map under the widget's key path, and outlives views. Give
  elements in lists and conditional places a `.key()` so their state
  follows them.
- **Elements own their data** (`text(format!(…))`, `Rc`/`Arc` for big
  shared data): a view borrows nothing from the program, so the kit can keep
  the last tree to route input to.

### `Cx`

What `launched` and `update` act with:

| | |
|---|---|
| `open_window(WindowSpec) -> WindowKey` | `WindowSpec::new(title).size(w, h).min_size(..).max_size(..).resizable(b)`, `.dialog(parent)`, `.utility()`, `WindowSpec::layer(LayerOptions, w, h)`, `WindowSpec::lock(output)`. Sizes are the content's: where the kit draws the menu bar in the window (no Aizome shell), the bar comes on top. |
| `close_window(key)`, `set_title(key, t)`, `request_size`, `minimise`, `set_fullscreen(key, on)`, `activate(key)`, `windows()`, `key_window()` | `activate` brings a window forward (a one-instance app hearing its second launch); `key_window` is the window with the keyboard (or the last that had it), the one a document app's menu commands are for. |
| `spawn(\|\| msg)` | Work on the kit's thread pool; its result comes back as a message. |
| `stream() -> MsgSender<Msg>` | A `Clone + Send` sender for threads that keep sending (D-Bus, IPC event streams, file watchers): `sender.send(msg)` wakes the loop. |
| `after(d, msg)`, `every(d, msg) -> TimerHandle`, `cancel(h)`, `send(msg)` | Timers; a message to the next update. |
| `clipboard_text()`, `set_clipboard_text(s)` | |
| `open_url(url)`, `notify(summary, body)` | xdg-open; a desktop notification. |
| `focus(name)` | Focus the element given `.id(name)` (now, or once it shows). |
| `theme() -> &Theme`, `quit()` | |
| `show_context_menu(window, at, Menu)` | A menu at a point. |
| `show_menu(window, anchor_rect, Menu, MenuStyle)` | A menu anchored to any rectangle of any window (the shell's bar menus). |
| `show_sheet(parent) -> WindowKey` | A sheet over a window: its content is `view(key)`; close it with `close_window`. |
| `alert(window, Alert)` | Title, message, buttons; Return and Escape press the default and cancel ones. |
| `open_panel(window, FileDialog, \|paths\| msg)`, `save_panel(…)` | File panels. |
| `outputs()`, `set_layer_size/margins/anchors/exclusive_zone/keyboard`, `set_input_region`, `set_opaque_region`, `lock_session()`, `unlock_session()`, `request_activation_token(app_id)` | The desktop's surfaces, for the shell. |

`Program` also has `menus()` (see Menus), `close_requested(window) -> bool`,
`open(paths)` (files from the command line), `window_state(window, state)`
(the window went full screen or back, by `set_fullscreen` or the desktop's
⌃❖F; maximised; told once per change, then viewed again), and, for the shell,
`outputs_changed`, `session_locked`, `session_lock_finished` and
`activation_token`.

## Elements

Every builder takes the common settings in any order with its own
(`Layoutable`, in the prelude): `.key(k)`, `.id(name)`,
`.width(len)`, `.height(len)`, `.min_width/.max_width/.min_height/.max_height`,
`.tooltip(text)`, `.label(text)` (for assistive technology).

Lengths: `Shrink` (the content's size, the default), `Fill` (all the room
there is; in a row or column, an equal share of what the others leave),
`Length::FillPortion(n)`, and points (`.width(200.0)`).

### Layout

| | |
|---|---|
| `column![a, b, c]`, `row![…]` | `.spacing(p)`, `.padding(p)` (a number, `[v, h]` or `[t, r, b, l]`), `.align(Align::Center)` across. Rows centre across by default. `aizome_kit::widgets::layout::column(vec)` for a built `Vec`. |
| `stack![base, over]` | Over one another, the last on top; `.align_x/.align_y`. |
| `container(el)` | `.padding`, `.fill(Tone::Chrome)`, `.border(Edge::Line)`, `.edges(Edge::BevelLo, Sides::BOTTOM)`, `.bevel()` (the top highlight), `.floating()` (raised, `bevel_lo` edge and the hard shadow), `.center()`, `.align_x/.align_y`, `.clip()`. |
| `scroll(el)` | `.horizontal()`, `.both()`. Momentum from touchpads, thin overlay bars that show while it moves and drag, Page Up/Down, Home/End, the arrows (when it has the focus, or nothing does). A control focused out of sight is scrolled into view. |
| `split(a, b)` | A divider to drag; `.vertical()`, `.ratio(r)`, `.first_size(p)`, `.min_sizes(a, b)`, `.on_resize(f)`. |
| `sidebar_layout(sidebar, content)` | Settings' and Files' layout: a 200pt `sunk` sidebar (draggable) beside the content. Fill it with `caption("System")` and `sidebar_item(Icon::Wifi, "Network", selected).on_press(msg).detail("3")`. |
| `spacer()`, `space(p)`, `divider()` | A stretch along a row or column; a fixed gap; a 1pt rule (`.vertical()`, `.engraved()`). |

### Text and pictures

| | |
|---|---|
| `text("…")` | `.role(Role::Text2)` (`Text`, `Text2`, `Text3`, `Faint`, `Accent`, `Danger`, `Success`, `Warning`, `Violet`, `Cyan`), `.size(p)`, `.weight(w)`, `.semibold()`, `.bold()`, `.mono()`, `.italic()`, `.tabular()`, `.condensed()` (Archivo at 87.5%), `.wrap()`, `.max_lines(n)` (wraps for up to `n` lines), `.center()`, `.caps()`, `.letter_spacing(p)`, `.underline()`, `.selectable()`. One line ends in an ellipsis where it doesn't fit. |
| `rich_text([Span::new("Ln ").role(Role::Text3), Span::new("48").mono()])` | Runs with their own look: `role`, `color`, `weight`, `italic`, `mono`, `underline`, `strikethrough`, `background`, `size`, `alpha` (the colour at part strength). |
| `icon(Icon::Wifi)` | `.size(16.0)`, `.role(Role::Text2)`. A curated set of ~290 Lucide icons (ISC), stroked at 1.5pt at 16pt. `Icon::from_name("wifi-off")`. |
| `image(Image)` | `.fit(Fit::Contain / Cover / Fill / None)`, `.revision(n)` when the picture changes at the same size. |
| `badge("3")`, `keycap("❖")` | A count on `red` (`.role(..)` for another); a key as a keycap. |

❖ ⇧ ⌥ ⌃ ⌘ ⎋ ⌫ ⇥ ↩ in any text are set in Noto Sans Symbols 2.

### Controls

All at least 24pt tall to the pointer, square, edged in 1pt; Tab reaches
them, Space and Return work them, arrows move within them.

| | |
|---|---|
| `button("Save")` | `.on_press(msg)`, `.primary()` (the selection fill), `.destructive()`, `.default()` (primary, answers Return in its sheet), `.cancel()` (answers Escape), `.icon(Icon)`, `.enabled(b)`, `.plain()`, `.compact()`, `.toolbar()` (both: a toolbar's B, I, </>), `.selected(b)` (pressed in), `.mono()`, `.weight(w)`, `.italic()`. `icon_button(Icon::Settings)`. |
| `toggle(on).on_toggle(Msg::Wifi)` | A switch. |
| `checkbox("Show hidden files", on).on_toggle(f)`, `radio("Night", value, Some(selected)).on_select(f)` | |
| `slider(v, 0.0..=1.0)` | `.step(s)`, `.ticks(n)`, `.on_change(f)`, `.on_release(msg)`. Arrows, Page Up/Down, Home/End. |
| `stepper(v).range(a..=b).step(s).suffix("pt").on_change(f)` | − value +. |
| `segmented(["Night", "Day", "Auto"], i).on_select(f)`, `segmented_icons([...], i)` | ← → change it. |
| `dropdown(options, selected).on_select(f)` | A pop-up button opening a menu of choices (the board's "Markdown ⌄"); `.placeholder(t)`, `.compact()` for toolbars. |
| `progress(f)`, `progress_indeterminate()`, `meter(values)` | The indeterminate bar ticks every 50 ms redrawing only itself, and stands still under reduced motion; meters are the menu bar's CPU bars (`.role(..)`, `.bar_width(..)`). |

### Text entry

`text_field(&self.name)` — `.placeholder(t)`, `.on_change(Msg::Name)`,
`.on_submit(msg)`, `.password()`, `.search()` (or `search_field(&q)`: a
magnifier, a clear button, Escape clears), `.error(Some("…"))` (a red edge
and the message under it), `.enabled(b)`, `.mono()`,
`.select_on_focus(range)` (what's selected when it first takes the focus:
a file name's stem, renaming it; key the field per use).

The program owns the text; the field keeps the caret, selection, input
method composition, scroll and undo history. Keys are macOS's with ❖ as
command: ←/→ by character, ⌥←/→ by word, ❖←/→ to the ends, ⇧ extends; ⌫
and ⌦, ⌥ by word, ❖⌫ to the start; ❖A ❖C ❖X ❖V ❖Z ⇧❖Z; double and triple
clicks select a word and everything. Typing undoes in bursts.

`text_editor(&self.content)` is the multi-line editor (Sumi's, Fusen's).
The program owns an `EditorContent` (a rope, the selection and the undo
history) and applies what the editor sends: `.on_action(Msg::Edit)`, then
`self.content.perform(action)` in `update`. `.on_find(Msg::Find)` hears
the Edit menu's Find items (`FindRequest`); the program draws its own find
bar and passes the matches back with `.marks(ranges, current)`. Also
`.line_numbers(b)`, `.wrap(b)`, `.mono(b)`, `.size(p)`, `.line_height(m)`
(21/13 by default), `.current_line(b)`, `.read_only()`, `.placeholder(t)`,
`.padding(p)`, and `.highlighter(Arc<dyn Fn(line, &str) -> Vec<LineStyle>>,
version)`, called for the lines that show. It lays out only those lines,
each through the text cache, and redraws only the lines that changed. The
gutter is the board's: numbers in `faint` right-aligned in a 30pt column,
14pt from the text, the caret's line's number in `text3` on a 10% `focus`
band (with a selection too). A selection the program sets (found text, a
line gone to) is scrolled into view, a third of the way down if it was far
out of sight.

### Collections

`virtual_list(count, row_height, |i| row(i))` builds only the rows that
show, from a closure that owns its data (an `Rc<Vec<…>>` clone):

```rust
let files = self.files.clone(); // Rc<Vec<File>>
virtual_list(files.len(), 24.0, move |i| row![icon(Icon::File), text(files[i].name.clone())].spacing(8.0))
    .selected(self.selected.iter().copied())
    .on_select(Msg::Select)        // Vec<usize>
    .on_activate(Msg::Open)        // double click, Return
    .multiple()                     // ⇧ and ❖ clicks
    .type_select(move |i| names[i].clone())
    .context_menu(|i| Menu::new("").item(MenuItem::new("Open", Msg::Open(i))))
```

`list(vec![...])` holds fixed rows. Selection belongs to the program; the
list draws it: on `selection`, with text and icons in `on_selection`, while
the list has the focus in the window with the keyboard; on `line2` in the
ordinary text colours otherwise. Arrows, Home/End, Page Up/Down and
type-select move it; the list scrolls to show it. `.reveal(row)` scrolls to
a row the program names (a search's match, a file just made) whenever the
row named changes, and leaves the user's scrolling alone otherwise.

`table(columns, rows, |r, c| cell(r, c))` is a list in columns:

```rust
let files = self.files.clone();
table(vec![Column::new("Name").width(240.0).sortable(), Column::new("Size").width(90.0).align_end()],
      files.len(), move |r, c| match c {
          0 => text(files[r].name.clone()).into(),
          _ => text(files[r].size_text()).mono().tabular().into(),
      })
    .selected(..).on_select(..).on_activate(..).multiple().type_select(..).context_menu(..)
    .sorted(0, true).on_sort(Msg::Sort) // a header clicked: (column, ascending)
```

The header stays at the top while rows scroll under it, and moves sideways
with them; its edges drag to resize columns (`.min_width(p)`; the last
column takes what's left), which the table remembers. Only the rows that
show are built.

`tree(Arc<Vec<TreeItem>>)` is an outline: `TreeItem::branch(id, label,
children)` and `TreeItem::leaf(id, label)`, each with `.icon(i)` and
`.detail(text)` (right-aligned, mono). `.expanded(Arc<HashSet<id>>)` and
`.on_toggle(Msg::Toggle)` let the program keep what's open (else the tree
keeps it); `.selected(ids)`, `.on_select`, `.on_activate`, `.multiple()`,
`.context_menu(|id| ..)`, `.reveal(id)`. → opens a branch, ← closes it or
moves to its parent, and its triangle opens and closes it. Rows are built
only while they show.

### Forms

```rust
form_section("Network", vec![
    form_row("Wi-Fi", toggle(self.wifi).on_toggle(Msg::Wifi)).icon(Icon::Wifi).into(),
    form_row("Size", dropdown(["Small", "Medium", "Large"], i).on_select(Msg::Size))
        .description("Scales type and controls, not pixels.").into(),
])
```

A mono caption, then a `chrome` box with a `line` edge and inset rules
between rows; labels left in `text2` (descriptions under them in `text3`),
controls right.

### Chrome

`toolbar(vec![...])` is the bar under a window's top edge (`chrome`, 30pt,
a `bevel_lo` bottom): compact pop-ups, `button(..).toolbar()`,
`toolbar_separator()`, `spacer()`, `status_text("Ln 48, Col 12")`.
`status_bar(vec![...])` runs along the bottom.

### Overlays and interaction

| | |
|---|---|
| `popover(anchor, content, open).on_dismiss(msg)` | Content in a floating popup under the anchor (raised, the hard shadow) while `open`; a click elsewhere or Escape sends `on_dismiss`. It takes the keyboard (Tab moves within it). |
| `context_menu(el, Menu)` | The menu on a right click (or ⌃-click). |
| `tooltip(el, "text")` (or `.tooltip(..)`) | After the pointer rests 650 ms. |
| `mouse_area(el)` | `.on_press`, `.on_release`, `.on_double_click`, `.on_right_click(\|at\| ..)`, `.on_hover(\|in\| ..)`, `.on_move`, `.on_drag(\|DragEvent\| ..)`, `.on_drop(\|Dropped { urls, text, .. }\| ..)` (files and text from other programs), `.cursor(c)`. |
| `canvas(state, \|canvas, bounds, &state, theme\| …)` | Drawing of one's own (charts, calendars, disk maps) with Sidestep's `Canvas`; it's drawn again when `state`'s hash changes. `.on_event(\|event, size, &state\| Option<Msg>)` (pointer, scroll, `Event::Pinch { scale, .. }` from a touchpad, keys), `.focusable()`, `.parts(\|&state, size\| vec![(rect, hash)])`: parts that change by themselves (a hovered bar, a cursor), left out of `state`'s `Hash`, so that only the rectangles of parts that came, went or changed are drawn again. |

### Sheets, alerts and panels

A sheet is a modal window of the program's own drawn over its parent's
content, under a dimming veil and at the top under the in-window menu bar:
`let s = cx.show_sheet(parent)`, then `view(s)` draws it and
`cx.close_window(s)` takes it away. While it's up the parent takes no
pointer or keys; Tab stays in the sheet, Return presses its `.default()`
button and Escape its `.cancel()` one.

```rust
cx.alert(window, Alert::new("Delete “notes.md”?", "It goes to the Trash.")
    .destructive_button("Delete", Msg::Delete)
    .cancel_button("Cancel"));          // or .default_button(..), .button(..), .cancel_with(.., msg)

cx.open_panel(window, FileDialog::new().title("Open").filter("Markdown", &["md"]).multiple(),
              Msg::Opened);             // Option<Vec<PathBuf>>: None when cancelled
cx.save_panel(window, FileDialog::new().file_name("Untitled.md"), Msg::SaveTo);
```

The file panel is the kit's own, as a sheet: a path bar of the folders up
to here, Places (Home, and Desktop, Documents, Downloads, Pictures, Music
and Videos where they exist, then Computer), the folder as a virtual list
(folders first, files by the filters; Return or a double click opens a
folder or chooses), a name field when saving, a "Show hidden files" box,
and `.directories()` to choose folders. The About role shows an alert with
the app's name, kind and version.

### Writing a widget

Implement `aizome_kit::widget::Widget` and wrap it with
`Element::new(widget, children)`. `layout` sizes it within constraints and
places children (`cx.layout_child`, `cx.place_child`); `paint` draws in its
own coordinates; `hash` feeds what its drawing depends on (the diff); `event`
answers input (`cx.emit(..)`, `cx.invalidate()`, `cx.relayout()`,
`cx.request_focus()`, `cx.request_tick(..)`, `cx.open_menu(..)`); its
persistent state is `cx.state::<T>()`. `aizome_kit::draw` has the look's
pieces: `raised`, `sunken`, `floating`, `border`, `separator`,
`focus_ring`.

## Keyboard

- Tab and ⇧Tab walk the focus chain (what can be used: disabled controls are
  skipped). A click focuses fields, lists and sliders; buttons take the
  focus only from the keyboard. The 2pt `focus` ring is drawn outside the
  focused control (always for text fields, when the focus came from the
  keyboard for others).
- Space or Return work the focused control. In a sheet, Return presses the
  default button and Escape the cancel one; Escape closes popovers and
  menus.
- Menus' shortcuts come before the focused widget: ❖ (Super) is the command
  key. With nothing focused, keys go to what's under the pointer (a scroll
  view pages).

## Menus

`Program::menus()` returns a `MenuBar`; the default is
`MenuBar::standard(&info)`:

- the app menu: About ‹Name›, Settings… ❖,, Services ▸, Hide ‹Name› ❖H,
  Hide Others ⌥❖H, Quit ‹Name› ❖Q;
- Edit: Undo ❖Z, Redo ⇧❖Z, Cut ❖X, Copy ❖C, Paste ❖V, Paste as Plain Text
  ⌥⇧❖V, Select All ❖A, Find ▸ (Find… ❖F, Find Next ❖G, Find Previous ⇧❖G,
  Use Selection for Find ❖E, Replace… ⌥❖F);
- Window: Minimise ❖M, Fill, Stack with…, Bring All to Front, and the
  windows;
- Help: Search, ‹Name› Help.

Extend it:

```rust
fn menus(&self) -> MenuBar<Msg> {
    MenuBar::standard(&self.info())
        .with_file_menu(Menu::new("File")
            .item(MenuItem::new("New", Msg::New).shortcut(Shortcut::cmd('N')))
            .item(MenuItem::new("Save", Msg::Save).shortcut(Shortcut::cmd('S')).enabled(self.dirty))
            .separator()
            .item(MenuItem::new("Export…", Msg::Export).shortcut(Shortcut::cmd('E').shift())))
        .with_menu(Menu::new("Format").item(MenuItem::new("Bold", Msg::Bold).checked(self.bold)))
}
```

`.replace(title, menu)`, `.remove(title)`, `.extend(title, items)`,
`.insert(i, menu)` change the standard ones. Items: `MenuItem::new(label,
msg)`, `.shortcut(Shortcut::cmd('N').shift().alt().ctrl())`, `.enabled`,
`.checked`, `.icon`, `.id(n)` (a stable id for the shell), `.role(MenuRole::..)`;
`MenuItem::role_item(label, role)` for an item the kit carries out itself;
`Menu::separator()`, `.submenu(menu)`, `.header(text)`.

Roles: the Edit roles go to the focused text widget (an item with a message
too gets it only when no widget takes the command); Quit asks each window's
`close_requested`; About shows the About panel; Minimise asks the window's
compositor; Hide, Hide Others, Fill, Stack with… and Bring All to Front go
through `Cx::set_window_manager` (the compositor's IPC).

Where `$AIZOME_SHELL_SOCKET` is set, the menus are published to the shell
(`docs/shell-protocol.md`) whenever they change; elsewhere the kit draws a
menu bar along each normal window's top.

Menus behave the same everywhere (in-window, the shell's, pop-up buttons,
context menus): ↑↓ move, → opens a submenu, ← closes it, Return or Space
chooses, typing filters the rows and their submenus' (flattened, with their
paths), Escape clears what's typed and then closes, and the pointer may
cross other rows on its way into an open submenu. `MenuStyle::at_point()
.titled("Edit").on_tear_off(msg)` gives NeXT's menus at the pointer their
`menu_title` strip, close box and pin; `menu_view(menu).title(..).pin(..)
.on_close(..)` draws a menu in a window, as a torn-off palette.

## Theming

The theme comes from `~/.config/aizome/settings.toml`'s `[appearance]`
(through `aizome-config`): `theme` (night, day, high-contrast, auto by the
clock), `focus` (amber, cyan, rose, lime), `effects` (full, lean,
austere: shadows and motion), `size` (small, medium, large: it scales type
and metrics, not pixels; targets never under 24pt), `reduced_motion`. The
kit watches the file and re-themes every window live. Night is the
default.

Widgets draw only with the theme: `cx.theme()` gives the palette as
Sidestep colours (`surface()`, `chrome()`, `raised()`, `sunk()`, `line()`,
`line2()`, `bevel_hi()`, `bevel_lo()`, `slab()`, `selection()`, `focus()`,
…), `role(Role)` for text, `metrics` (`body`, `small`, `control`, `row`,
`menu_row`, `toolbar`, `icon`, `gap`, `pad`, …) and `font(FontSpec)`:
Archivo for the interface, IBM Plex Mono for code, shortcuts and numbers
(tabular), Noto Sans Symbols 2 for the modifier glyphs.

Fonts: at start the kit installs, for the process, the families fontconfig
doesn't have from `$AIZOME_FONT_DIR`, else `../share/aizome/fonts` beside
the executable; without them the system's sans and mono stand in.

## Performance

The budget is 4 ms from an event to its frame drawn, at 2880 × 1800 (scale 2).

- An event is handled by the widget under the pointer (or the focused one)
  and its parents; hover, press and focus changes invalidate only the
  widget's own rectangle and call no view.
- A message runs `update`, then `view` for each window, then layout, then
  the diff: only rectangles of nodes that moved, resized or draw
  differently are invalidated, and painting skips every node outside the
  damage (`canvas.needs`).
- Scrolling, splits and lists re-lay-out without a view: the widget asks
  (`cx.relayout()`), the kit lays out it and its ancestors again (siblings
  keep their cached layout) and diffs.
- Text layouts are cached by text, style and width (an LRU of 4096), so
  views rebuilt every frame shape nothing new.
- Virtual lists build the rows that show; a 10,000-row list costs ~15
  rows.

Rules for apps: keep `view` cheap (no I/O, no sorting of big data: sort in
`update`), share big data with `Rc`/`Arc`, key elements that move, use
`virtual_list` for anything long, and `spawn` slow work.

The numbers measured on this laptop are at the end of this document.

## Testing an app headlessly

`aizome_kit::testing::Harness` runs a program on a stand-in platform: windows
open at the size asked for, timers run on a clock the test moves, the
clipboard is kept, and pictures come from Sidestep's offscreen renderer (the
same rasterizer and text engine windows use).

```rust
use aizome_kit::testing::{Harness, assert_golden};

#[test]
fn adds() {
    let mut h = Harness::new(Soroban::default());
    let w = h.window();
    h.click_text(w, "7");
    h.type_text(w, "+2\n");
    h.command(w, 'c');                         // ❖C
    h.named(w, NamedKey::Tab, Modifiers::NONE);
    assert!(h.texts(w).contains(&"9".to_owned()));
    assert_eq!(h.platform.clipboard.as_deref(), Some("9"));
    h.advance(Duration::from_secs(1));         // timers
    assert_golden(&mut h, w, "tests/snapshots/soroban.png", 1.0);
}
```

`Harness::with(program, appearance, menu_bar)` picks the theme and the
in-window menu bar. Input: `click`, `click_text`, `press`/`release`,
`drag`, `move_to`, `scroll`, `pinch`, `key`, `named`, `command`, `type_text`, `ime`,
`focus_window`, `send(msg)`, `resize`, `window_state` (the compositor's
answer; `fullscreen_asked` says what was asked). Looking: `find(text)`,
`find_id(name)`, `texts()`, `semantics()`, `selection()`,
`focused_name()`, `popups()`, `take_damage()`, `title()`, `stats()`.
Pictures: `render(window, scale)`, `render_rect`, `render_popup`,
`render_rgba` (the window with its popups where they'd show), `save_png`,
and `assert_golden` (`AIZOME_UPDATE_GOLDENS=1` rewrites them). Popups:
`popups(window)`, `click_popup`, `popup_keyboard`, `texts_in`. Drops:
`drop(window, at, urls, text)`. The theme: `set_appearance`. For the shell:
`set_outputs`, `confirm_lock`, `finish_lock`, `activation_token`, and
`Harness::with_shell(program, socket)` with `pump_shell(wait)` to talk to a
stand-in shell over a real socket.

`crates/aizome-kit/examples/gallery` shows every element on five pages
(Controls, Text & Fields, Lists & Tables, a Settings pane, the Editor) with
the theme switchable; the kit's goldens in `tests/snapshots` are its pages.
`GALLERY_DEMO=menu|popover|alert` opens one at start, and
`GALLERY_DEMO=bench-toggle|bench-select|bench-type` keeps changing
something (for `SIDESTEP_FRAME_STATS=1`).

Set `AIZOME_FONT_DIR` to the fonts (`~/work/.aizome-fonts` here) so
pictures use Aizome's type.

To run an app's real windows without touching a live session, run it under
the headless sway (`sidestep-aizome/scripts/headless-wayland`) with
`WAYLAND_DISPLAY` and `DISPLAY` unset first.

## Accessibility

Every widget describes itself (`Widget::semantics`: role, label, value,
checked, selected, range, actions) and the kit builds a semantic tree per
window (`aizome_kit::a11y`; `Harness::semantics` in tests). Wiring it to
AccessKit (and AT-SPI) is to come.

## Measured

On this laptop (release build, the machine shared with a dozen busy
builds: load average 10–15, so the maxima are the scheduler's), at 1440 ×
900 points rendered at scale 2 (2880 × 1800 pixels).

From the input to the frame drawn, in the headless harness
(`cargo test --release -p aizome-kit --test perf -- --ignored --nocapture`;
the time includes handling, `update`, `view`, layout, the diff, and
rasterizing the damage into an offscreen buffer):

| Interaction | p50 ms | p95 ms |
|---|---|---|
| Hover a button (a settings pane, ~200 widgets) | 0.01 | 0.01 |
| Toggle a switch (update, view, layout, diff) | 0.04 | 0.14 |
| Type a character in a text field | 0.08 | 0.12 |
| Type a character in a 2,000-line editor | 0.35 | 0.71 |
| Move a 10,000-row list's selection by a row | 0.12 | 0.16 |
| Scroll a 10,000-row list one notch, the list filling the window | 3.8 | 28 |
| (The offscreen buffer for a whole 2880 × 1800 frame alone) | 1.2 | 2.2 |

The live render thread, under headless sway at scale 2
(`SIDESTEP_FRAME_STATS=1`, the gallery's `GALLERY_DEMO=bench-…` modes,
one change every 20 ms; record, rasterize and present, from Sidestep's
frame statistics):

| Change | Damage | p50 µs | p95 µs |
|---|---|---|---|
| A switch | 1% | 47 | 64 |
| A 10,000-row table's and a list's selection | 4–5% | 220–350 | 370–440 |
| Typing in the editor | 2–3% | 130–200 | 220–240 |

Everything but scrolling is far inside 4 ms because only what changed is
drawn. A notch of scrolling redraws the whole list, so its cost is the
area's: a list filling a 2880 × 1800 window misses 4 ms at the 95th
percentile while the machine is busy. Sidestep is making full-window
repaints at 2× faster; moving the drawn rows (a blit) and drawing only the
newly shown strip would make scrolling proportional to the rows that come
into view.
