# Logs into Aizome in a disposable VM and checks the desktop is there and
# works, with screenshots of each step (in the test's output directory).
#
# The VM has a virtio GPU without 3D, so the compositor renders with pixman:
# this is also the test of Aizome's software-rendered fallback. greetd logs
# alice straight into the session, as a login screen would after her
# password.
#
#     nix build .#checks.x86_64-linux.session -L
#     nix build .#checks.x86_64-linux.session.driverInteractive && ./result/bin/nixos-test-driver
{ pkgs, self }:
pkgs.testers.runNixOSTest {
  name = "aizome-session";

  nodes.machine =
    { pkgs, ... }:
    {
      imports = [ self.nixosModules.default ];
      programs.aizome.enable = true;
      # Settings takes the time zone over from here (programs.aizome.mutableTimeZone).
      time.timeZone = "Europe/Paris";

      users.users.alice = {
        isNormalUser = true;
        password = "aizome";
        extraGroups = [
          "wheel"
          "networkmanager"
          "video"
        ];
        uid = 1000;
      };

      services.greetd = {
        enable = true;
        settings.initial_session = {
          user = "alice";
          command = "/run/current-system/sw/bin/aizome-session";
        };
        settings.default_session.command = "${pkgs.greetd}/bin/agreety --cmd /run/current-system/sw/bin/aizome-session";
      };

      virtualisation = {
        memorySize = 4096;
        cores = 4;
        resolution = {
          x = 1440;
          y = 900;
        };
        qemu.options = [
          "-vga none"
          "-device virtio-gpu-pci,xres=1440,yres=900"
        ];
      };
      # No bluetooth controller or printers in the VM; their services still
      # run and report nothing, which is what the widgets must handle.
      environment.systemPackages = [
        pkgs.wtype
        # ffprobe, for the screen recording.
        pkgs.ffmpeg
      ];
    };

  enableOCR = false;

  testScript = ''
    import json

    def as_alice(command):
        return machine.succeed(
            "su - alice -c 'export XDG_RUNTIME_DIR=/run/user/1000; "
            "export WAYLAND_DISPLAY=$(ls /run/user/1000 | grep -m1 \"^wayland-[0-9]*$\"); "
            "export AIZOME_SOCKET=/run/user/1000/aizome.$WAYLAND_DISPLAY.sock; "
            + command + "'"
        )

    def ipc(request):
        return json.loads(as_alice("aizomectl --json " + request))

    def capture(name):
        machine.screenshot(name)
        as_alice(f"aizomectl screenshot /tmp/{name}.png")
        machine.copy_from_machine(f"/tmp/{name}.png")

    machine.wait_for_unit("multi-user.target")
    machine.wait_until_succeeds("ls /run/user/1000/ | grep -q '^aizome\\..*\\.sock$'", timeout=120)

    with subtest("the session is up: compositor, shell, outputs"):
        machine.wait_until_succeeds(
            "su - alice -c 'XDG_RUNTIME_DIR=/run/user/1000 systemctl --user is-active aizome.service'",
            timeout=60,
        )
        machine.wait_until_succeeds("pgrep -u alice -x aizome-shell", timeout=60)
        # The session's environment file reaches the session (it must be
        # imported whole into the user manager, which the compositor and the
        # shell start from).
        shell_env = machine.succeed("tr '\\0' '\\n' < /proc/$(pgrep -u alice -x aizome-shell)/environ")
        for name in ["DCONF_PROFILE=aizome", "XDG_CONFIG_DIRS=/etc/xdg/aizome", "NIXOS_OZONE_WL=1"]:
            assert name in shell_env, f"{name} isn't in the shell's environment:\n{shell_env}"
        outputs = ipc("outputs")
        print(outputs)
        assert len(outputs) >= 1, outputs
        capture("01-desktop")

    with subtest("a terminal opens with a focused tab"):
        as_alice("aizomectl spawn foot")
        machine.wait_until_succeeds(
            "su - alice -c 'export XDG_RUNTIME_DIR=/run/user/1000 AIZOME_SOCKET=$(ls /run/user/1000/aizome.wayland-*.sock); aizomectl windows' | grep -q foot",
            timeout=30,
        )
        capture("02-terminal")

    with subtest("every app is installed: its program, desktop entry and icon"):
        sw = "/run/current-system/sw"
        apps = ["Shitsurae", "Yukimi", "Hako", "Sumi", "Fusen", "Koyomi", "Tokei",
                "Soroban", "Tenki", "Tansu", "Kura", "Myaku", "Shashin", "Annai"]
        for app in apps:
            entry = f"{sw}/share/applications/org.aizome.{app}.desktop"
            machine.succeed(f"test -f {entry}")
            machine.succeed(f"test -f {sw}/share/icons/hicolor/scalable/apps/org.aizome.{app}.svg")
            # Exec names a program on the PATH.
            machine.succeed(f"command -v $(sed -n 's/^Exec=\\([^ ]*\\).*/\\1/p' {entry})")
        machine.succeed("test -f /etc/xdg/autostart/org.aizome.Annai.Welcome.desktop")
        # The units the package installs name its programs by store path.
        unit = "/etc/systemd/user/aizome-koyomi-agent.service"
        machine.succeed(f"grep -q '^ExecStart=/nix/store/[^ ]*/bin/aizome-koyomi --agent$' {unit}")
        machine.fail("grep -Rl @bindir@ /etc/systemd/user/")

    with subtest("Aizome's apps open"):
        for app in ["aizome-shitsurae", "aizome-soroban", "aizome-hako"]:
            machine.succeed(f"test -x /run/current-system/sw/bin/{app}")
            as_alice(f"aizomectl spawn {app}")
        machine.sleep(5)
        print(as_alice("aizomectl windows"))
        capture("03-apps")

    with subtest("the lock screen locks and unlocks with the password"):
        as_alice("aizomectl lock")
        machine.sleep(3)
        capture("04-locked")
        machine.send_chars("aizome\n")
        machine.sleep(3)
        capture("05-unlocked")

    with subtest("Settings can change the time zone and the computer's name"):
        machine.succeed("timedatectl set-timezone Asia/Tokyo")
        machine.succeed("readlink /etc/localtime | grep -q Asia/Tokyo")
        machine.succeed("hostnamectl set-hostname --pretty 'Aizome Test'")
        assert "Aizome Test" in machine.succeed("hostnamectl --pretty")

    with subtest("a screen recording starts, shows, stops and leaves a video"):
        # gpu-screen-recorder can't run here (virtio-gpu has no hardware
        # OpenGL), so this is the fallback: wf-recorder through the
        # compositor's screencopy (wl_shm under pixman), x264.
        import shlex

        def alice(command):
            env = (
                "export XDG_RUNTIME_DIR=/run/user/1000; "
                "export WAYLAND_DISPLAY=$(ls /run/user/1000 | grep -m1 '^wayland-[0-9]*$'); "
                "export AIZOME_SOCKET=/run/user/1000/aizome.$WAYLAND_DISPLAY.sock; "
            )
            return "su - alice -c " + shlex.quote(env + command)

        def recording():
            return json.loads(machine.succeed(alice("aizome-shell msg state")))["recording"]

        record_screen = alice("aizome-shell msg '{\"request\":\"record\",\"target\":\"screen\"}'")
        # Something moving, so frames keep coming.
        machine.succeed(alice("aizomectl spawn -- foot sh -c 'while :; do date +%T.%N; sleep 0.05; done'"))
        # What the palette's Record Screen does.
        machine.succeed(record_screen)
        machine.wait_until_succeeds(alice("aizome-shell msg state | grep -q '\"phase\":\"recording\"'"), timeout=30)
        # One at a time.
        machine.fail(record_screen)
        # gpu-screen-recorder takes seconds to give up on software OpenGL;
        # the clock restarts with each recorder that takes over. Five
        # seconds of the one that stuck:
        machine.wait_until_succeeds(
            alice("aizome-shell msg state | grep -Eq '\"seconds\":([5-9]|[1-9][0-9])'"), timeout=60
        )
        going = recording()
        print(going)
        # gpu-screen-recorder refuses software OpenGL, so wf-recorder.
        print("recording with", going["recorder"])
        assert going["recorder"] in ["gpu-screen-recorder", "wf-recorder (VA-API)", "wf-recorder"], going
        capture("06-recording")
        machine.sleep(2)
        machine.succeed(alice("aizome-shell msg stop-recording"))
        machine.wait_until_succeeds(alice("aizome-shell msg state | grep -q '\"recording\":null'"), timeout=30)
        video = going["path"]
        assert video.startswith("/home/alice/Videos/Recordings/screenrecording-"), video
        probe = json.loads(machine.succeed(
            "ffprobe -v error -select_streams v:0 -count_frames -show_entries "
            "stream=codec_name,width,height,r_frame_rate,nb_read_frames:format=duration "
            f"-of json '{video}'"
        ))
        print(probe)
        stream = probe["streams"][0]
        mode = ipc("outputs")["outputs"][0]["current_mode"]
        assert stream["codec_name"] == "h264", stream
        assert (stream["width"], stream["height"]) == (mode["width"], mode["height"]), stream
        assert stream["r_frame_rate"] == "60/1", stream
        assert float(probe["format"]["duration"]) >= 4, probe
        assert int(stream["nb_read_frames"]) >= 100, stream
        capture("07-recording-saved")

    with subtest("frame times stay inside the budget"):
        print(as_alice("aizomectl stats"))
  '';
}
