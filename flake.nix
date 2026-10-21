{
  description = "Aizome: a desktop environment for Sekka, on Sidestep's native toolkit";

  inputs = {
    # The same nixpkgs as the systems Aizome is developed on, so the build
    # shares their store paths.
    nixpkgs.url = "github:NixOS/nixpkgs/7fd4004d0511991fff3a0524856a9be09e28c5f0";
    # Sidestep's native toolkit, from the fork's `aizome` branch. The
    # workspace depends on it by path (../sidestep-aizome), so the package
    # places this source beside Aizome's.
    sidestep = {
      url = "github:bitemyapp/sidestep/aizome";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      sidestep,
    }:
    let
      lib = nixpkgs.lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      # Libraries the compositor (smithay), the toolkit (Sidestep) and the
      # apps link against or load at run time.
      buildInputs =
        pkgs: with pkgs; [
          wayland
          libxkbcommon
          libinput
          udev
          seatd
          libdrm
          mesa
          libgbm
          libglvnd
          pixman
          fontconfig
          freetype
          dbus
          pam
          # aizome-sys talks to PipeWire through pipewire-pulse.
          libpulseaudio
          libsecret
        ];
      # Only what the build reads: the workspace and the files the package
      # installs.
      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./crates
          ./icons
          ./etc
          ./scripts
          (lib.fileset.maybeMissing ./apps)
          (lib.fileset.maybeMissing ./session)
        ];
      };
    in
    {
      packages = forAll (pkgs: rec {
        aizome = pkgs.callPackage ./nix/package.nix {
          inherit src sidestep;
          version = "0.1.0-${self.shortRev or self.dirtyShortRev or "dirty"}";
          rev = self.rev or self.dirtyRev or null;
        };
        archivo = pkgs.callPackage ./nix/fonts.nix { };
        # The session's fonts (Archivo, and nixpkgs' IBM Plex Mono and Noto
        # Sans Symbols 2) as files in one folder, for $AIZOME_FONT_DIR
        # (docs/kit.md).
        fonts = pkgs.callPackage ./nix/aizome-fonts.nix { inherit archivo; };
        default = aizome;
      });

      nixosModules.default = import ./nix/module.nix { inherit self; };
      nixosModules.aizome = self.nixosModules.default;

      # Boots a VM, logs into the Aizome session and checks the desktop is
      # up (nix/vm-test.nix).
      checks = forAll (pkgs: {
        session = import ./nix/vm-test.nix { inherit pkgs self; };
        # Changes something in every pane of Settings and checks the
        # system took it (nix/vm-settings-test.nix).
        settings = import ./nix/vm-settings-test.nix { inherit pkgs self; };
        timezone-policy = import ./nix/timezone-policy-test.nix { inherit pkgs self; };
        # The login screen: people, sessions (another Wayland one, an X11
        # one), a wrong password, logging out and back (nix/vm-greeter-test.nix).
        greeter = import ./nix/vm-greeter-test.nix { inherit pkgs self; };
        greeter-x11 = import ./nix/vm-greeter-x11-test.nix { inherit pkgs self; };
        koyomi-session = import ./nix/vm-koyomi-session-test.nix { inherit pkgs self; };
        koyomi-modal = import ./nix/vm-koyomi-modal-test.nix { inherit pkgs self; };
        network-auth = import ./nix/vm-network-auth-test.nix { inherit pkgs self; };
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            just
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
            pkg-config
            clang
            mold
          ];
          buildInputs = buildInputs pkgs;
          # dlopen'd at run time: EGL/GL drivers, fontconfig, xkbcommon.
          LD_LIBRARY_PATH = lib.makeLibraryPath (buildInputs pkgs);
          RUSTFLAGS = "-C link-arg=-fuse-ld=mold";
          # The compositor's core is a Rust dylib that needs -C prefer-dynamic
          # (docs/reload.md); the wrapper adds it for that crate alone.
          RUSTC_WRAPPER = "${./scripts/rustc-wrapper}";
        };
      });
    };
}
