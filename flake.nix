{
  description = "Aizome: a desktop environment for Sekka, on Sidestep's native toolkit";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/7fd4004d0511991fff3a0524856a9be09e28c5f0";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      # Libraries the compositor (smithay), the toolkit (Sidestep) and the
      # apps link against or load at run time.
      buildInputs = pkgs: with pkgs; [
        wayland libxkbcommon libinput udev seatd libdrm mesa libgbm libglvnd pixman
        fontconfig freetype dbus pam
      ];
    in {
      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ rustc cargo clippy rustfmt rust-analyzer pkg-config clang mold ];
          buildInputs = buildInputs pkgs;
          # dlopen'd at run time: EGL/GL drivers, fontconfig, xkbcommon.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (buildInputs pkgs);
          RUSTFLAGS = "-C link-arg=-fuse-ld=mold";
        };
      });
    };
}
