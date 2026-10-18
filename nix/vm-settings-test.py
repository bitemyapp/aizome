# The script of nix/vm-settings-test.nix: drive Settings the way a person
# would and check each change reached the system. WIFI_PASSPHRASE comes
# from the Nix file. AIZOME_TEST_ONLY=appearance,sound runs only the parts
# whose titles hold those words (when the driver runs outside the sandbox).
import json
import os
import re
import shlex
import time

RUN = "/run/user/1000"
INSPECT = RUN + "/inspect"
APP = "org.aizome.Shitsurae"
HOME = "/home/alice"


def alice(command, check=True):
    """Run a command as alice in her session (its bus, its compositor)."""
    script = (
        "export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus DCONF_PROFILE=aizome; "
        "export WAYLAND_DISPLAY=$(ls /run/user/1000 | grep -m1 '^wayland-[0-9]*$'); "
        "export AIZOME_SOCKET=/run/user/1000/aizome.$WAYLAND_DISPLAY.sock; " + command
    )
    cmd = "su - alice -c " + shlex.quote(script)
    if check:
        return machine.succeed(cmd)
    return machine.execute(cmd)


def ipc(request):
    return json.loads(alice("aizomectl --json " + request))


def setting(path):
    """A value from alice's settings.toml, by its dotted path, or None."""
    script = (
        "import tomllib, json, sys\n"
        "try:\n    d = tomllib.load(open(sys.argv[1], 'rb'))\nexcept FileNotFoundError:\n    d = {}\n"
        "for k in sys.argv[2].split('.'):\n    d = d.get(k) if isinstance(d, dict) else None\n"
        "print(json.dumps(d))"
    )
    out = machine.succeed(f"python3 -c {shlex.quote(script)} {HOME}/.config/aizome/settings.toml {shlex.quote(path)}")
    return json.loads(out)


def settings_toml():
    return machine.succeed(f"cat {HOME}/.config/aizome/settings.toml 2>/dev/null || true")


def shot(name):
    machine.screenshot(name)


def until(what, check, timeout=20, every=0.5):
    """Wait for check() to return something truthy, and return it."""
    deadline = time.monotonic() + timeout
    last = None
    while True:
        try:
            last = check()
        except Exception as e:  # noqa: BLE001 (a read racing a write)
            last = e
        if last and not isinstance(last, Exception):
            return last
        if time.monotonic() > deadline:
            shot("failed-" + "".join(c if c.isalnum() else "-" for c in what)[:60])
            raise AssertionError(f"timed out waiting for {what}: {last!r}")
        time.sleep(every)


# Input through the compositor, as a person's.


def move(x, y):
    alice(f"aizomectl input motion {x:.1f} {y:.1f}")


def click_at(x, y, button="left", times=1):
    clicks = " && ".join([f"aizomectl input click {button}"] * times)
    alice(f"aizomectl input motion {x:.1f} {y:.1f} && {clicks}")


def key(name):
    alice(f"aizomectl input key {name}")


def chord(keys):
    alice(f"aizomectl input chord {keys}")


def type_text(text):
    alice("aizomectl input type " + shlex.quote(text))


def select_all_and_type(text):
    chord("super+a")
    type_text(text)


class Settings:
    """Settings (Shitsurae), as the kit's inspection files describe it."""

    CONTROLS = [
        "switch",
        "checkbox",
        "slider",
        "combobox",
        "button",
        "radiobutton",
        "textinput",
        "searchinput",
        "passwordinput",
        "spinbutton",
        "tab",
    ]

    def __init__(self):
        self.pid = None

    def running(self):
        return machine.execute("pgrep -u alice '^aizome-shitsura'")[0] == 0

    def open(self, pane):
        """Open Settings at a pane, as the palette or a systray panel does."""
        assert not self.running(), "Settings is already open"
        alice(f"mkdir -p {INSPECT} && rm -f {INSPECT}/{APP}.*")
        alice(f"aizomectl spawn env AIZOME_KIT_INSPECT={INSPECT} aizome-shitsurae --pane {pane}")
        self.pid = int(
            until("Settings to start", lambda: machine.succeed("pgrep -u alice '^aizome-shitsura' || true").strip())
        )
        until("Settings' window", lambda: self.window() and self.doc())
        # Its first frames settle while the pane reads the system.
        time.sleep(1.5)
        return self

    def quit(self):
        """Quit with ❖Q, as a person would, and wait for it to go."""
        self.focus()
        chord("super+q")
        until("Settings to quit", lambda: not self.running(), timeout=15)
        self.pid = None

    def focus(self):
        w = self.window()
        assert w, "no Settings window"
        if not w["focused"]:
            ipc(f"focus {w['id']}")
            until("Settings to be focused", lambda: self.window()["focused"])

    def window(self):
        for w in ipc("windows")["windows"]:
            if w["app_id"] == APP and w["pid"] == self.pid and w.get("parent") is None:
                return w
        return None

    def docs(self):
        out = machine.succeed(
            f'for f in {INSPECT}/{APP}.{self.pid}.*.json; do [ -f "$f" ] && cat "$f" && echo; done; true'
        )
        return [json.loads(line) for line in out.splitlines() if line.strip()]

    def doc(self, kind="window"):
        for d in self.docs():
            if d["kind"] == kind:
                return d
        return None

    def elements(self):
        d = self.doc()
        return d["elements"] if d else []

    # Finding.

    @staticmethod
    def matches(e, text=None, name=None, role=None, label=None, contains=None, inside=None, visible=True):
        if visible and not e["visible"]:
            return False
        if text is not None and e.get("text") != text:
            return False
        if contains is not None and contains not in (e.get("text") or e.get("label") or e.get("value") or ""):
            return False
        if name is not None and e.get("name") != name:
            return False
        if role is not None and e.get("role") not in (role if isinstance(role, (list, tuple)) else [role]):
            return False
        if label is not None and e.get("label") != label:
            return False
        if inside is not None:
            x, y, w, h = e["rect"]
            X, Y, W, H = inside["rect"]
            if not (x >= X - 0.5 and y >= Y - 0.5 and x + w <= X + W + 0.5 and y + h <= Y + H + 0.5):
                return False
        return True

    def all(self, **kw):
        return [e for e in self.elements() if self.matches(e, **kw)]

    def find(self, **kw):
        found = self.all(**kw)
        return found[0] if found else None

    def wait(self, what=None, timeout=20, **kw):
        return until(what or f"an element {kw}", lambda: self.find(**kw), timeout=timeout)

    def gone(self, what=None, timeout=20, **kw):
        until(what or f"no element {kw}", lambda: self.doc() is not None and self.find(**kw) is None, timeout=timeout)

    def row(self, pane, entry):
        """An option's row (form rows are named entry:<pane>.<entry>)."""
        return self.find(name=f"entry:{pane}.{entry}", visible=False)

    def control(self, pane, entry, role=None, **kw):
        """The control in an option's row, scrolled into view."""
        self.reveal(name=f"entry:{pane}.{entry}")
        row = self.row(pane, entry)
        assert row, f"no row {pane}.{entry}"
        found = self.find(inside=row, role=role or self.CONTROLS, **kw)
        if not found:
            inside = [e for e in self.elements() if self.matches(e, inside=row, visible=False)]
            raise AssertionError(f"no control in row {pane}.{entry}: {inside}")
        return found

    def texts(self, inside=None):
        return [
            e["text"]
            for e in self.elements()
            if e.get("text") and (inside is None or self.matches(e, inside=inside, visible=False))
        ]

    # Looking around: wheel the pane until something shows whole.

    def reveal(self, tries=30, **kw):
        for _ in range(tries):
            e = until(f"something like {kw}", lambda: self.find(visible=False, **kw), timeout=10)
            x, y, w, h = e["rect"]
            sx, sy, sw, sh = e["shown"]
            if e["visible"] and sh >= min(h, 30) - 0.5 and sw > 0:
                return e
            d = self.doc()
            height = d["size"][1]
            win = self.window()["geometry"]
            # Over the pane (right of the 232pt sidebar), wheel towards it.
            px = win["x"] + 232 + (win["w"] - 232) / 2
            py = win["y"] + win["h"] / 2
            dy = 3 if y > height / 2 else -3
            alice(f"aizomectl input motion {px:.1f} {py:.1f} && aizomectl input scroll 0 {dy}")
            time.sleep(0.4)
        raise AssertionError(f"couldn't scroll {kw} into view")

    # Acting.

    def point(self, e):
        win = self.window()["geometry"]
        x, y, w, h = e["shown"] if e["visible"] else e["rect"]
        return win["x"] + x + w / 2, win["y"] + y + h / 2

    def click(self, e=None, button="left", times=1, **kw):
        if e is None:
            self.reveal(**kw)
            e = self.wait(**kw)
        self.focus()
        x, y = self.point(e)
        click_at(x, y, button, times)
        time.sleep(0.5)
        return e

    def press(self, label, **kw):
        """Click the button (or card, or row) a person knows by this name."""
        return self.click(self.button(label, **kw))

    def button(self, label, **kw):
        """A button (or card, or row) by the name a person knows it by, in view."""
        for want in ({"role": "button", "label": label}, {"label": label}, {"text": label}):
            if self.find(visible=False, **want, **kw):
                return self.reveal(**want, **kw)
        raise AssertionError(f"no button {label!r}")

    def slide(self, e, fraction):
        """Drag a slider's knob to a fraction of its track."""
        self.focus()
        win = self.window()["geometry"]
        x, y, w, h = e["rect"]
        lo, cur, hi = e["range"]
        start = win["x"] + x + w * ((cur - lo) / (hi - lo) if hi > lo else 0)
        to = win["x"] + x + w * fraction
        cy = win["y"] + y + h / 2
        alice(
            f"aizomectl input motion {start:.1f} {cy:.1f} && aizomectl input button left down && "
            f"aizomectl input motion {(start + to) / 2:.1f} {cy:.1f} && aizomectl input motion {to:.1f} {cy:.1f} && "
            "aizomectl input button left up"
        )
        time.sleep(0.6)

    def choose(self, popup, item):
        """Open a pop-up button and pick an item by typing its name."""
        self.click(popup)
        until("the menu to open", lambda: self.doc("menu"))
        type_text(item)
        key("return")
        until("the menu to close", lambda: self.doc("menu") is None)
        time.sleep(0.5)

    def field(self, e, text, submit=True):
        """Replace a text field's text (and press Return)."""
        self.click(e)
        chord("super+a")
        type_text(text)
        if submit:
            key("return")
        time.sleep(0.6)

    def beside(self, label, role, **kw):
        """The control on the same line as some text (a row without an entry)."""
        t = self.reveal(text=label)
        cy = t["rect"][1] + t["rect"][3] / 2
        best = None
        for e in self.all(role=role, visible=False, **kw):
            ey = e["rect"][1] + e["rect"][3] / 2
            if e["rect"][0] > t["rect"][0] and abs(ey - cy) < 16:
                if best is None or abs(ey - cy) < abs(best["rect"][1] + best["rect"][3] / 2 - cy):
                    best = e
        assert best, f"no {role} beside {label!r}"
        return best

    def segment(self, control, name):
        """Click a segment of a segmented control by its name."""
        for p in control.get("parts", []):
            if p["name"] == name:
                win = self.window()["geometry"]
                x, y, w, h = p["rect"]
                self.focus()
                click_at(win["x"] + x + w / 2, win["y"] + y + h / 2)
                time.sleep(0.5)
                return
        raise AssertionError(f"no segment {name!r} in {control}")

    def checked(self, pane, entry, role="switch"):
        return self.control(pane, entry, role=role).get("checked")


S = Settings()


def shell_docs():
    pid = machine.succeed("pgrep -u alice -x aizome-shell").split()[0]
    out = machine.succeed(
        f'for f in {INSPECT}/org.aizome.Shell.{pid}.*.json; do [ -f "$f" ] && cat "$f" && echo; done; true'
    )
    return [json.loads(line) for line in out.splitlines() if line.strip()]


def shell_texts():
    return [e["text"] for d in shell_docs() for e in d["elements"] if e.get("text") and e["visible"]]


def shell_surface(holding):
    """The texts of the shell's surface that shows `holding` (the bar: Toolchest)."""
    for d in shell_docs():
        texts = [e["text"] for e in d["elements"] if e.get("text") and e["visible"]]
        if holding in texts:
            return texts
    return []


def locked():
    """The shell's lock screen shows (its password field)."""
    return any(e.get("role") == "passwordinput" for d in shell_docs() for e in d["elements"])


def shell_labels():
    return [e["label"] for d in shell_docs() for e in d["elements"] if e.get("label") and e["visible"]]


TESTS = []


def part(title):
    def add(f):
        TESTS.append((title, f))
        return f

    return add


def reopen(pane):
    S.quit()
    return S.open(pane)


# Appearance.


@part("Appearance: Day and the rose focus colour reach the config, dconf and GTK; Night comes back")
def appearance():
    S.open("appearance")
    S.press("Day")
    until("theme = day", lambda: setting("appearance.theme") == "day")
    S.press("Rose")
    until("focus = rose", lambda: setting("appearance.focus") == "rose")
    # GTK apps: libadwaita reads color-scheme (and the portal relays it),
    # GTK 3 the theme's name.
    until(
        "dconf to say light",
        lambda: "prefer-light" in alice("dconf read /org/gnome/desktop/interface/color-scheme"),
    )
    portal = alice(
        "gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop "
        "--method org.freedesktop.portal.Settings.ReadOne org.freedesktop.appearance color-scheme"
    )
    assert "uint32 2" in portal, f"the portal doesn't say light: {portal}"
    gtk = alice("dconf read /org/gnome/desktop/interface/gtk-theme")
    assert "Adwaita" in gtk and "dark" not in gtk, f"GTK 3 stays dark: {gtk}"
    accent = alice("dconf read /org/gnome/desktop/interface/accent-color")
    assert "pink" in accent, f"no accent for GTK: {accent!r}"
    shot("appearance-day")
    reopen("appearance")
    day = S.button("Day")
    assert day.get("selected") or day.get("checked"), f"Day isn't shown chosen: {day}"
    rose = S.button("Rose")
    assert rose.get("selected") or rose.get("checked"), f"Rose isn't shown chosen: {rose}"
    S.press("Night")
    until("theme = night", lambda: setting("appearance.theme") == "night")
    until("dconf to say dark", lambda: "prefer-dark" in alice("dconf read /org/gnome/desktop/interface/color-scheme"))
    S.quit()


@part("Desktop & Deskbar: a Desk renamed and added, the Shelf and Trash hidden, tabs minimise on double-click")
def desktop():
    S.open("desktop")
    S.field(S.control("desktop", "desks", role="textinput"), "Mail")
    until("the compositor's first Desk to be Mail", lambda: ipc("desks")["desks"][0]["name"] == "Mail")
    assert setting("desks")[0]["name"] == "Mail", settings_toml()
    count = len(ipc("desks")["desks"])
    S.press("Add a Desk")
    until("a Desk more", lambda: len(ipc("desks")["desks"]) == count + 1)

    until("the Deskbar's Shelf", lambda: "Shelf" in shell_texts())
    S.click(S.control("desktop", "shelf", role="switch"))
    until("show_shelf = false", lambda: setting("deskbar.show_shelf") is False)
    until("the Deskbar to drop the Shelf", lambda: "Shelf" not in shell_texts())

    until("the desktop's Trash", lambda: "Trash" in shell_texts())
    S.click(S.beside("Trash", "switch"))
    until("show_trash = false", lambda: setting("desktop.show_trash") is False)
    until("the desktop to drop the Trash", lambda: "Trash" not in shell_texts())

    S.choose(S.control("desktop", "double-click", role="combobox"), "Minimise")
    until("tab_double_click = minimise", lambda: setting("windows.tab_double_click") == "minimise")
    # Double-click Settings' own tab, above its frame's top left.
    w = S.window()
    g = w["geometry"]
    click_at(g["x"] + 60, g["y"] - 12, times=2)
    until("Settings to minimise", lambda: S.window()["minimised"])
    ipc(f"unminimise {w['id']}")
    until("Settings back", lambda: not S.window()["minimised"])

    reopen("desktop")
    assert S.control("desktop", "desks", role="textinput").get("value") == "Mail"
    assert S.checked("desktop", "shelf") is False
    assert S.beside("Trash", "switch").get("checked") is False
    assert S.control("desktop", "double-click", role="combobox").get("value") == "Minimise"
    # Back as they were.
    S.click(S.control("desktop", "shelf", role="switch"))
    S.click(S.beside("Trash", "switch"))
    S.choose(S.control("desktop", "double-click", role="combobox"), "Fill the screen")
    until("the Shelf back", lambda: "Shelf" in shell_texts())
    S.quit()


@part("Menu Bar: seconds on the clock and the Bluetooth widget off reach the shell's bar")
def menubar():
    S.open("menubar")
    S.click(S.control("menubar", "clock-seconds", role="switch"))
    until("clock_seconds = true", lambda: setting("menubar.clock_seconds") is True)
    until(
        "the bar's clock to show seconds",
        lambda: any(re.fullmatch(r"\d{1,2}:\d{2}:\d{2}.*", t) for t in shell_texts()),
    )
    before = shell_labels()
    print("the bar's labels:", before)
    S.click(S.control("menubar", "widget-bluetooth", role="switch"))
    until("widgets without bluetooth", lambda: "bluetooth" not in (setting("menubar.widgets") or []))
    until("the bar without Bluetooth", lambda: "Bluetooth" not in shell_texts())
    reopen("menubar")
    assert S.checked("menubar", "clock-seconds") is True
    assert S.checked("menubar", "widget-bluetooth") is False
    S.click(S.control("menubar", "clock-seconds", role="switch"))
    S.click(S.control("menubar", "widget-bluetooth", role="switch"))
    until("widgets with bluetooth", lambda: "bluetooth" in (setting("menubar.widgets") or []))
    S.quit()


@part(
    "Date & Time: the 12-hour clock and Tokyo's time zone reach the shell and timedated; NixOS's network time is explained"
)
def datetime_():
    S.open("general.datetime")
    assert S.checked("general", "clock-24h") is True
    S.click(S.control("general", "clock-24h", role="switch"))
    until("clock = 12h", lambda: setting("menubar.clock") == "12h")
    until("the bar's clock in 12 hours", lambda: any(re.search(r"\b(AM|PM)\b", t) for t in shell_texts()))

    # NixOS keeps systemd's units in the store, so timedated can't turn
    # network time on or off: the row says how it's set instead.
    ntp = machine.succeed("timedatectl show -p NTP --value").strip() == "yes"
    auto = " ".join(S.texts(S.row("general", "auto-time")))
    assert "services.timesyncd.enable" in auto and ("On" if ntp else "Off") in auto, auto

    city = S.find(role=["searchinput", "textinput"], label="City, country or zone", visible=False)
    S.field(S.reveal(role=["searchinput", "textinput"], label="City, country or zone"), "Tokyo", submit=False)
    S.click(S.button("Tokyo"))
    until(
        "the zone to be Tokyo's",
        lambda: machine.succeed("timedatectl show -p Timezone --value").strip() == "Asia/Tokyo",
    )
    assert city is not None

    reopen("general.datetime")
    assert S.checked("general", "clock-24h") is False
    zone = " ".join(S.texts(S.row("general", "timezone")))
    assert "Tokyo" in zone, zone
    S.click(S.control("general", "clock-24h", role="switch"))
    until("clock = 24h", lambda: setting("menubar.clock") == "24h")
    S.quit()


@part("About: the computer's name reaches hostnamed; the host name NixOS sets is explained")
def about():
    S.open("general.about")
    S.field(S.control("general", "computer-name", role="textinput"), "Alice's Aizome")
    until("the pretty name", lambda: machine.succeed("hostnamectl --pretty").strip() == "Alice's Aizome")
    host = " ".join(S.texts(S.row("general", "hostname")))
    assert "networking.hostName" in host, host
    reopen("general.about")
    assert S.control("general", "computer-name", role="textinput").get("value") == "Alice's Aizome"
    S.quit()


@part("Default Apps: the text editor chosen opens text files")
def default_apps():
    S.open("general.default-apps")
    S.choose(S.control("general", "default-text-editor", role="combobox"), "Plain Editor")
    until(
        "mimeapps.list to name it",
        lambda: "text/plain=aizome-test-editor.desktop" in machine.succeed(f"cat {HOME}/.config/mimeapps.list || true"),
    )
    query = alice(
        "XDG_CURRENT_DESKTOP=Aizome XDG_CONFIG_DIRS=/etc/xdg/aizome:/etc/xdg xdg-mime query default text/plain"
    )
    assert query.strip() == "aizome-test-editor.desktop", query
    reopen("general.default-apps")
    assert S.control("general", "default-text-editor", role="combobox").get("value") == "Plain Editor"
    S.quit()


@part("Login Items: the welcome tour stops opening at login")
def login_items():
    S.open("general.login-items")
    switch = S.beside("Welcome to Aizome", "switch")
    assert switch.get("checked") is True
    S.click(switch)
    until(
        "a hidden autostart entry",
        lambda: (
            "Hidden=true" in machine.succeed(f"cat {HOME}/.config/autostart/org.aizome.Annai.Welcome.desktop || true")
        ),
    )
    reopen("general.login-items")
    assert S.beside("Welcome to Aizome", "switch").get("checked") is False
    S.quit()


@part("Language & Region: NixOS sets the locale, and the pane says so")
def language():
    S.open("general.language")
    text = " ".join(S.texts(S.row("general", "system-language")))
    assert "Set by your system configuration" in text, text
    S.quit()


@part("Lock Screen: a minute's idleness locks; the password unlocks; Never stays")
def lock():
    S.open("lock")
    S.choose(S.control("lock", "lock-after", role="combobox"), "1 minute")
    until("lock_after_minutes = 1", lambda: setting("idle.lock_after_minutes") == 1)
    S.click(S.control("lock", "show-clock", role="switch"))
    until("show_clock = false", lambda: setting("lock.show_clock") is False)
    S.quit()
    # Nobody touches anything for a minute.
    until("the screen to lock", locked, timeout=110, every=2)
    shot("lock-locked")
    machine.send_chars("aizome\n")
    until("the screen to unlock", lambda: not locked(), timeout=20)
    S.open("lock")
    assert S.control("lock", "lock-after", role="combobox").get("value") == "1 minute"
    assert S.checked("lock", "show-clock") is False
    S.choose(S.control("lock", "lock-after", role="combobox"), "Never")
    until("lock_after_minutes = 0", lambda: setting("idle.lock_after_minutes") == 0)
    S.click(S.control("lock", "show-clock", role="switch"))
    S.quit()


@part("Notifications: Do Not Disturb holds banners back, and a quiet schedule is kept")
def notifications():
    S.open("notifications")
    S.click(S.control("notifications", "dnd", role="switch"))
    until("do_not_disturb = true", lambda: setting("notifications.do_not_disturb") is True)
    S.click(S.control("notifications", "schedule", role="switch"))
    until("quiet hours", lambda: setting("notifications.quiet_from") == "22:00")
    S.segment(S.control("notifications", "banner-time", role="tablist"), "12 s")
    until("banner_seconds = 12", lambda: setting("notifications.banner_seconds") == 12)
    alice("notify-send 'Quiet please' 'Held back by Do Not Disturb'")
    # Kept for the Deskbar, not shown in the bar.
    until("the Deskbar to keep it", lambda: any("Quiet please" in t for t in shell_texts()))
    time.sleep(1)
    assert not any("Quiet please" in t for t in shell_surface("Toolchest")), shell_surface("Toolchest")
    reopen("notifications")
    assert S.checked("notifications", "dnd") is True
    assert S.checked("notifications", "schedule") is True
    assert S.control("notifications", "banner-time", role="tablist").get("value") == "12 s"
    S.click(S.control("notifications", "dnd", role="switch"))
    until("do_not_disturb = false", lambda: setting("notifications.do_not_disturb") is False)
    alice("notify-send 'Hello again' 'Shown now'")
    until("the bar to show it", lambda: any("Hello again" in t for t in shell_surface("Toolchest")))
    S.quit()


@part("Accessibility: reduce motion, sticky keys and a larger pointer are kept")
def accessibility():
    S.open("accessibility")
    S.click(S.control("accessibility", "reduced-motion", role="switch"))
    until("reduced_motion = true", lambda: setting("appearance.reduced_motion") is True)
    S.click(S.control("accessibility", "sticky-keys", role="switch"))
    until("sticky_keys = true", lambda: setting("input.keyboard.sticky_keys") is True)
    S.slide(S.control("accessibility", "cursor-size", role="slider"), 0.75)
    until("a larger pointer", lambda: (setting("appearance.cursor_size") or 24) > 24)
    reopen("accessibility")
    assert S.checked("accessibility", "reduced-motion") is True
    assert S.checked("accessibility", "sticky-keys") is True
    S.click(S.control("accessibility", "reduced-motion", role="switch"))
    S.click(S.control("accessibility", "sticky-keys", role="switch"))
    until("sticky_keys = false", lambda: setting("input.keyboard.sticky_keys") is False)
    S.quit()


@part("Performance: the renderer for the next login is kept and the compositor knows")
def performance():
    S.open("performance")
    use = S.control("performance", "renderer", role="tablist")
    S.segment(use, "Software")
    until("renderer = software", lambda: setting("performance.renderer") == "software")
    stats = ipc("stats")
    print(stats)
    S.segment(S.control("performance", "renderer", role="tablist"), "GPU")
    until("renderer = gpu", lambda: setting("performance.renderer") == "gpu")
    stats = ipc("stats")["stats"]
    assert stats.get("renderer_setting") == "gpu", stats
    assert stats.get("renderer_after_restart") is True, stats
    reopen("performance")
    assert S.control("performance", "renderer", role="tablist").get("value") == "GPU"
    S.segment(S.control("performance", "renderer", role="tablist"), "Automatic")
    until("renderer = auto", lambda: setting("performance.renderer") == "auto")
    S.quit()


@part("Search: a palette scope turned off is kept")
def search():
    S.open("search")
    S.click(S.control("search", "scopes", role="switch"))
    until("a scope less", lambda: setting("palette.scopes") is not None and "commands" not in setting("palette.scopes"))
    reopen("search")
    assert S.checked("search", "scopes") is False
    S.click(S.control("search", "scopes", role="switch"))
    S.quit()


@part("Users & Groups: a new full name reaches AccountsService and the password file")
def users():
    S.open("users")
    S.field(S.control("users", "name", role="textinput"), "Alice Pleasance Liddell")
    until(
        "AccountsService's name",
        lambda: (
            "Alice Pleasance Liddell"
            in machine.succeed(
                "busctl get-property org.freedesktop.Accounts /org/freedesktop/Accounts/User1000 org.freedesktop.Accounts.User RealName"
            )
        ),
    )
    assert "Alice Pleasance Liddell" in machine.succeed("getent passwd alice")
    reopen("users")
    assert S.control("users", "name", role="textinput").get("value") == "Alice Pleasance Liddell"
    S.quit()


# The session, then each part.

machine.wait_for_unit("multi-user.target")
machine.wait_until_succeeds("ls /run/user/1000/ | grep -q '^aizome\\..*\\.sock$'", timeout=120)
machine.wait_until_succeeds("pgrep -u alice -x aizome-shell", timeout=60)
# The welcome tour stays out of the way.
machine.sleep(3)
for w in ipc("windows")["windows"]:
    if w["app_id"].startswith("org.aizome.Annai"):
        ipc(f"close {w['id']}")

with subtest("the session is up"):
    print(json.dumps(ipc("outputs")["outputs"], indent=1))
    shot("00-desktop")

ONLY = [w.strip().lower() for w in os.environ.get("AIZOME_TEST_ONLY", "").split(",") if w.strip()]
# Commands to look around with first (in development: AIZOME_TEST_PROBE,
# separated by ;;).
for probe in [p for p in os.environ.get("AIZOME_TEST_PROBE", "").split(";;") if p.strip()]:
    print("probe:", probe)
    print(machine.execute(probe)[1])
for title, test in TESTS:
    if ONLY and not any(w in title.lower() for w in ONLY):
        continue
    with subtest(title):
        try:
            test()
        except Exception:
            shot("failed-" + test.__name__)
            print(machine.execute(f"ls -la {INSPECT}")[1])
            try:
                print("the shell shows:", shell_texts())
            except Exception as e:  # noqa: BLE001
                print("no shell:", e)
            if S.running() and S.pid:
                print(json.dumps(S.doc(), indent=None)[:20000])
            raise
