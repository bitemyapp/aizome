# Aizome as a NixOS module (this flake's nixosModules.default):
# `programs.aizome.enable = true;` adds the Aizome session to the login
# screen, with what its shell and apps stand on: the portals, the services
# behind Settings (NetworkManager, BlueZ, UPower, power profiles, UDisks2,
# AccountsService, CUPS, geoclue), the lock screen's PAM service, the fonts,
# and foot themed for Aizome. Everything session-specific is read only in
# Aizome sessions (/etc/xdg/aizome, the `aizome` dconf profile), so other
# desktops on the same machine keep their own look.
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.aizome;
  system = pkgs.stdenv.hostPlatform.system;
  archivo = pkgs.callPackage ./fonts.nix { };
  # The login entry runs the session from the system profile rather than a
  # store path, so an update switches without logging out and the next
  # login starts the new Aizome (as Sekka's Tatami entry does).
  session = pkgs.writeTextFile {
    name = "aizome-session-entry";
    destination = "/share/wayland-sessions/aizome.desktop";
    text = ''
      [Desktop Entry]
      Name=Aizome
      Comment=Sekka's desktop: Haiku tabs, NeXT menus and IRIX's Toolchest, in indigo
      Exec=/run/current-system/sw/bin/aizome-session
      Type=Application
      DesktopNames=Aizome
    '';
    derivationArgs.passthru.providedSessions = [ "aizome" ];
  };
  # wf-recorder (the fallback recorder) puts its wl_shm buffers in files in
  # /tmp, which on most NixOS systems is on disk: every frame the compositor
  # copies would be written out (over 1 GB/s at 2880 × 1800 and 60 fps) and
  # the compositor would wait on the disk. In memfds they stay in memory.
  wf-recorder = pkgs.wf-recorder.overrideAttrs (old: {
    postPatch = (old.postPatch or "") + ''
      substituteInPlace src/main.cpp \
        --replace-fail 'int fd = mkstemp(name);' 'int fd = memfd_create("wf-recorder-shared", MFD_CLOEXEC);'
    '';
  });
  # Read by aizome-session before it starts the compositor: what every
  # program in the session sees.
  environment = {
    XDG_CURRENT_DESKTOP = "Aizome";
    XDG_SESSION_DESKTOP = "aizome";
    XDG_SESSION_TYPE = "wayland";
    XDG_CONFIG_DIRS = "/etc/xdg/aizome:/etc/xdg";
    DCONF_PROFILE = "aizome";
    NIXOS_OZONE_WL = "1";
    MOZ_ENABLE_WAYLAND = "1";
    QT_QPA_PLATFORM = "wayland;xcb";
    QT_QPA_PLATFORMTHEME = "gtk3";
    SDL_VIDEODRIVER = "wayland,x11";
    _JAVA_AWT_WM_NONREPARENTING = "1";
    AIZOME_WALLPAPER_NIGHT = toString cfg.wallpapers.night;
    AIZOME_WALLPAPER_DAY = toString cfg.wallpapers.day;
  };
in
{
  options.programs.aizome = {
    enable = lib.mkEnableOption "the Aizome desktop session";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${system}.aizome;
      defaultText = lib.literalExpression "aizome.packages.\${system}.aizome";
      description = "The Aizome package: compositor, shell, aizomectl and apps.";
    };
    mutableTimeZone = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Let Settings (and the automatic time zone) change the time zone.
        Aizome takes `time.timeZone` over, starting from the zone the
        system was last set to, so changes made in Settings last.
      '';
    };
    videoAcceleration = lib.mkOption {
      type = lib.types.bool;
      default = pkgs.stdenv.hostPlatform.isx86_64;
      defaultText = lib.literalExpression "pkgs.stdenv.hostPlatform.isx86_64";
      description = ''
        Install Intel's VA-API driver (intel-media-driver), so screen
        recordings encode on Intel GPUs (and video players decode there)
        rather than on the CPU. AMD's driver comes with Mesa, NVIDIA's
        encoder with its driver.
      '';
    };
    alarmsWakeSystem = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Let Tokei's alarms wake the computer from sleep (CAP_WAKE_ALARM for user services).";
    };
    wallpapers = {
      night = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        example = lib.literalExpression ''"''${tatami-wallpapers}/share/backgrounds/tatami/da-nang-at-night.jpg"'';
        description = "The Night theme's default wallpaper. Without one the desktop is kachi, the near-black indigo.";
      };
      day = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "The Day theme's default wallpaper.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [
      cfg.package
      pkgs.foot
      pkgs.xwayland-satellite
      pkgs.wl-clipboard
      pkgs.xdg-utils
      pkgs.adwaita-icon-theme
      pkgs.hicolor-icon-theme
      pkgs.shared-mime-info
      # GPU names for Activity and About (aizome-sys's sysinfo).
      pkgs.hwdata
      # Keyboard layouts and options for Settings (evdev.xml), linked
      # into /run/current-system/sw/share/X11/xkb.
      pkgs.xkeyboard_config
      # Hako compresses and extracts archives with bsdtar.
      pkgs.libarchive
      # Tokei rings with pw-play and the freedesktop sound theme.
      pkgs.pipewire
      pkgs.sound-theme-freedesktop
      pkgs.libnotify
      # Screen recording where gpu-screen-recorder can't run (no hardware
      # OpenGL, as in a VM): through the compositor's wlr-screencopy. Ahead
      # of another desktop's (Tatami installs it too).
      (lib.hiPrio wf-recorder)
    ];
    # Screen recording (the palette's Record Screen and Record Area, ⌥Print):
    # gpu-screen-recorder, with the capability wrapper its KMS capture needs
    # (gsr-kms-server, CAP_SYS_ADMIN), as Tatami has it.
    programs.gpu-screen-recorder.enable = lib.mkDefault true;
    # The GPU's drivers (OpenGL for the compositor, VA-API for recordings)
    # in /run/opengl-driver.
    hardware.graphics.enable = lib.mkDefault true;
    hardware.graphics.extraPackages = lib.mkIf cfg.videoAcceleration [ pkgs.intel-media-driver ];
    services.displayManager.sessionPackages = [ session ];
    systemd.packages = [ cfg.package ];

    # Under load, the session (the compositor, which also weighs 1000 within
    # it, pipewire, the portals) runs before apps, so the desktop keeps
    # drawing and answering and Ctrl+Alt+Del opens at once. Raising
    # session.slice rather than lowering app.slice keeps apps ahead of
    # background.slice (30): 400 : 100 : 30.
    systemd.user.slices.session.sliceConfig = {
      CPUWeight = lib.mkDefault 400;
      IOWeight = lib.mkDefault 400;
    };

    # In Aizome, Settings owns the time zone and the computer's name:
    # Sekka lets its desktop configure more than a desktop usually does.
    # The time zone starts from the one the system is set to now (NixOS's
    # link while time.timeZone was set), saved before /etc is rebuilt and
    # put back if the rebuild removed it; after that timedated keeps it.
    time.timeZone = lib.mkIf cfg.mutableTimeZone (lib.mkOverride 90 null);
    system.activationScripts = lib.mkIf cfg.mutableTimeZone {
      aizomeTimeZoneSave.text = ''
        if zone=$(readlink -f /etc/localtime 2>/dev/null) && [ -f "$zone" ]; then
          case "$zone" in
            */zoneinfo/*)
              mkdir -p /var/lib/aizome
              printf '%s\n' "''${zone#*/zoneinfo/}" > /var/lib/aizome/timezone
              ;;
          esac
        fi
      '';
      etc.deps = [ "aizomeTimeZoneSave" ];
      # The link is written as timedated writes it, relative. An absolute
      # link to /etc/zoneinfo is NixOS's own for time.timeZone, which
      # Settings reads as the configuration's; one left behind from before
      # (time.timeZone is null here) is turned into Settings' kind.
      aizomeTimeZoneRestore = {
        deps = [ "etc" ];
        text = ''
          zone=
          if target=$(readlink /etc/localtime 2>/dev/null); then
            case "$target" in
              /etc/zoneinfo/*) zone=''${target#/etc/zoneinfo/} ;;
            esac
          elif [ ! -e /etc/localtime ] && [ -s /var/lib/aizome/timezone ]; then
            zone=$(cat /var/lib/aizome/timezone)
          fi
          if [ -n "$zone" ] && [ -e "/etc/zoneinfo/$zone" ]; then
            ln -sfn "../etc/zoneinfo/$zone" /etc/localtime
          fi
        '';
      };
    };
    # The pretty name ("The studio laptop") in a file hostnamed can write;
    # the host name itself stays networking.hostName.
    systemd.services.systemd-hostnamed.environment.SYSTEMD_ETC_MACHINE_INFO = lib.mkForce "/etc/machine-info";

    # Tokei's alarms are transient systemd timers, which end with the user
    # manager: schedule them again at each login, in any desktop.
    systemd.user.services.aizome-tokei-alarms = {
      description = "Schedule Tokei's alarms";
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${cfg.package}/bin/aizome-tokei --sync";
      };
    };
    # Koyomi's alerts, as notifications, whether or not Koyomi is open
    # (the unit comes with the package).
    systemd.user.services.aizome-koyomi-agent.wantedBy = [ "graphical-session.target" ];
    # An alarm can wake the computer (Tokei's "Wake the computer"), which
    # needs CAP_WAKE_ALARM for the user's timers.
    systemd.services."user@".serviceConfig.AmbientCapabilities = lib.mkIf cfg.alarmsWakeSystem [
      "CAP_WAKE_ALARM"
    ];

    environment.etc = {
      "xdg/aizome/environment".text = lib.concatStrings (
        lib.mapAttrsToList (name: value: "${name}=${value}\n") (
          lib.filterAttrs (_: value: value != "") environment
        )
      );
      "xdg/aizome/foot/foot.ini".source = ../etc/foot/foot.ini;
      # Aizome's apps open what they're for in Aizome sessions: the
      # desktop-specific list (XDG_CURRENT_DESKTOP=Aizome) is read before
      # mimeapps.list, so other desktops keep their own defaults, and the
      # user's own choices (~/.config/mimeapps.list) still win.
      "xdg/aizome-mimeapps.list".text =
        let
          entry = app: types: lib.concatMapStrings (type: "${type}=org.aizome.${app}.desktop;\n") types;
        in
        ''
          [Default Applications]
        ''
        + entry "Shashin" [
          "image/png"
          "image/apng"
          "image/jpeg"
          "image/gif"
          "image/webp"
          "image/bmp"
          "image/tiff"
          "image/x-icon"
          "image/vnd.microsoft.icon"
          "image/svg+xml"
        ]
        + entry "Sumi" [
          "text/plain"
          "text/markdown"
          "text/x-markdown"
          "text/rust"
          "text/x-python"
          "text/x-nix"
          "application/x-shellscript"
          "application/toml"
          "application/json"
          "application/x-yaml"
        ]
        + entry "Hako" [ "inode/directory" ]
        + entry "Koyomi" [ "text/calendar" ]
        + entry "Tansu" [
          "application/x-cd-image"
          "application/x-raw-disk-image"
        ];
      # The welcome tour, once, at the first Aizome login (it decides for
      # itself whether to show; OnlyShowIn=Aizome keeps it out of others).
      "xdg/autostart/org.aizome.Annai.Welcome.desktop".source =
        "${cfg.package}/share/applications/org.aizome.Annai.Welcome.desktop";
    };

    # Screenshots and screencasts through wlr-screencopy; files, settings
    # and the rest through GTK's portal.
    xdg.portal = {
      enable = true;
      extraPortals = [
        pkgs.xdg-desktop-portal-wlr
        pkgs.xdg-desktop-portal-gtk
      ];
      configPackages = [ cfg.package ];
    };
    # Terminal applications open in foot in Aizome sessions.
    xdg.terminal-exec = {
      enable = true;
      settings.Aizome = [ "foot.desktop" ];
    };

    # The lock screen authenticates through PAM as this service.
    security.pam.services.aizome = { };
    security.polkit.enable = true;

    # What Settings, the Deskbar's widgets and the apps talk to.
    networking.networkmanager.enable = lib.mkDefault true;
    hardware.bluetooth.enable = lib.mkDefault true;
    services.upower.enable = lib.mkDefault true;
    services.power-profiles-daemon.enable = lib.mkDefault true;
    services.udisks2.enable = lib.mkDefault true;
    services.accounts-daemon.enable = lib.mkDefault true;
    services.printing.enable = lib.mkDefault true;
    # Network and driverless (IPP Everywhere) printers announce themselves
    # over mDNS, which is how Settings finds them.
    services.avahi = {
      enable = lib.mkDefault true;
      nssmdns4 = lib.mkDefault true;
    };
    services.geoclue2.enable = lib.mkDefault true;
    # Tenki's "Current Location", and Settings' automatic time zone. They
    # ask for city accuracy; the location comes from nixpkgs' default
    # provider (BeaconDB).
    services.geoclue2.appConfig = {
      "org.aizome.Tenki" = {
        isAllowed = true;
        isSystem = false;
      };
      "org.aizome.Shitsurae" = {
        isAllowed = true;
        isSystem = false;
      };
    };
    services.gvfs.enable = lib.mkDefault true;
    services.gnome.gnome-keyring.enable = lib.mkDefault true;
    services.pipewire = {
      enable = lib.mkDefault true;
      pulse.enable = lib.mkDefault true;
      wireplumber.enable = lib.mkDefault true;
    };
    security.rtkit.enable = lib.mkDefault true;

    # GTK and Qt applications follow Aizome's Night in its sessions: the
    # `aizome` dconf profile (selected by DCONF_PROFILE above) holds the
    # defaults; the user's own settings still win.
    programs.dconf.enable = true;
    programs.dconf.profiles.aizome.databases = [
      {
        settings."org/gnome/desktop/interface" = {
          color-scheme = "prefer-dark";
          gtk-theme = "Adwaita-dark";
          icon-theme = "Adwaita";
          cursor-theme = "Adwaita";
          cursor-size = lib.gvariant.mkInt32 24;
          font-name = "Archivo 11";
          document-font-name = "Archivo 11";
          monospace-font-name = "IBM Plex Mono 11";
          clock-format = "24h";
        };
        settings."org/gnome/desktop/wm/preferences".button-layout = "";
      }
    ];

    fonts.packages = [
      archivo
      pkgs.ibm-plex
      pkgs.noto-fonts
      pkgs.noto-fonts-cjk-sans
      pkgs.noto-fonts-cjk-serif
      pkgs.noto-fonts-color-emoji
    ];
  };
}
