# Aizome: the compositor, the shell, aizomectl and the apps, from one build
# of the workspace.
#
# The workspace depends on Sidestep's `sidestep-ui` by path
# (../sidestep-aizome/crates/sidestep-ui), so the source here is a directory
# holding this repository and the Sidestep fork side by side, as they sit
# on a developer's disk. The fork's revision is pinned by the flake's
# `sidestep` input.
{
  lib,
  rustPlatform,
  runCommand,
  pkg-config,
  makeWrapper,
  wayland,
  libxkbcommon,
  libinput,
  udev,
  seatd,
  libdrm,
  libgbm,
  libglvnd,
  mesa,
  pixman,
  fontconfig,
  freetype,
  dbus,
  libpulseaudio,
  pam,
  src,
  sidestep,
  version ? "0.1.0",
}:
let
  source = runCommand "aizome-source" { } ''
    mkdir -p $out
    cp -r ${src} $out/aizome
    cp -r ${sidestep} $out/sidestep-aizome
  '';
  # Libraries opened at run time with dlopen rather than linked: EGL and
  # GLES through libglvnd (the drivers come from /run/opengl-driver),
  # fontconfig for the text engine, and xkbcommon for smithay's keymaps.
  runtimeLibs = [
    libglvnd
    libpulseaudio
    fontconfig
    libxkbcommon
    wayland
  ];
in
rustPlatform.buildRustPackage {
  pname = "aizome";
  inherit version;
  src = source;
  sourceRoot = "aizome-source/aizome";
  cargoLock.lockFile = "${src}/Cargo.lock";

  nativeBuildInputs = [
    pkg-config
    makeWrapper
    rustPlatform.bindgenHook
  ];
  buildInputs = [
    wayland
    libxkbcommon
    libinput
    udev
    seatd
    libdrm
    libgbm
    libglvnd
    mesa
    pixman
    fontconfig
    freetype
    dbus
    pam
    libpulseaudio
  ];

  cargoBuildFlags = [ "--workspace" ];
  # The tests need a Wayland display, D-Bus services or a GPU; they run in
  # the VM test and in development, not in the sandbox.
  doCheck = false;

  postInstall = ''
    # Session: the login entry, the systemd user units and the portals.
    install -Dm755 session/aizome-session $out/bin/aizome-session
    install -Dm644 session/aizome.desktop $out/share/wayland-sessions/aizome.desktop
    for unit in session/*.service session/*.target; do
      install -Dm644 "$unit" "$out/lib/systemd/user/$(basename "$unit")"
    done
    install -Dm644 session/aizome-portals.conf $out/share/xdg-desktop-portal/aizome-portals.conf

    # Apps: desktop entries and icons.
    for entry in apps/*/data/*.desktop; do
      [ -e "$entry" ] && install -Dm644 "$entry" "$out/share/applications/$(basename "$entry")"
    done
    for icon in icons/apps/*.svg; do
      install -Dm644 "$icon" "$out/share/icons/hicolor/scalable/apps/$(basename "$icon")"
    done
    # The Toolchest's mark, for the shell and the login screen.
    install -Dm644 icons/apps/org.aizome.Toolchest.svg $out/share/aizome/mark.svg
  '';

  postFixup = ''
    for bin in $out/bin/*; do
      if isELF "$bin"; then
        patchelf --add-rpath ${lib.makeLibraryPath runtimeLibs} "$bin"
      fi
    done
  '';

  passthru.providedSessions = [ "aizome" ];

  meta = {
    description = "Aizome, Sekka's desktop environment, on Sidestep's native Rust toolkit";
    homepage = "https://github.com/bitemyapp/aizome";
    license = with lib.licenses; [
      mit
      asl20
    ];
    mainProgram = "aizome";
    platforms = lib.platforms.linux;
  };
}
