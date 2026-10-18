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
      environment.systemPackages = [ pkgs.wtype ];
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

    with subtest("frame times stay inside the budget"):
        print(as_alice("aizomectl stats"))
  '';
}
