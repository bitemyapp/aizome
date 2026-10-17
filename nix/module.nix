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
    ];
    services.displayManager.sessionPackages = [ session ];
    systemd.packages = [ cfg.package ];

    environment.etc = {
      "xdg/aizome/environment".text = lib.concatStrings (
        lib.mapAttrsToList (name: value: "${name}=${value}\n") (
          lib.filterAttrs (_: value: value != "") environment
        )
      );
      "xdg/aizome/foot/foot.ini".source = ../etc/foot/foot.ini;
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
    services.geoclue2.enable = lib.mkDefault true;
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
