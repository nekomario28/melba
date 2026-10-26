{
  description = "Apple Music, native and fast";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # rust-toolchain.toml pins the compiler so local builds and CI agree.
    # This reads that file rather than restating the version here.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ (import rust-overlay) ];
            }
          )
        );
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages =
            with pkgs;
            [
              (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
              rust-analyzer
              pkg-config
            ]
            ++ lib.optionals stdenv.hostPlatform.isDarwin [
              apple-sdk
            ]
            ++ lib.optionals stdenv.hostPlatform.isLinux [
              dbus
              alsa-lib
              libpulseaudio
              libxkbcommon
              wayland
              libGL
              libx11
              libxcursor
              libxi
              libxrandr
            ];
          # The GUI dlopens its Wayland, X11 and GL libraries at run time.
          LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (
            pkgs.lib.makeLibraryPath (
              with pkgs;
              [
                dbus
                libxkbcommon
                wayland
                libGL
                libx11
                libxcursor
                libxi
                libxrandr
              ]
            )
          );
        };
      });

      packages = forAllSystems (
        pkgs:
        let
          melba =
            let
              toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
              rustPlatform = pkgs.makeRustPlatform {
                cargo = toolchain;
                rustc = toolchain;
              };
              runtimeLibs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
                with pkgs;
                [
                  dbus
                  libxkbcommon
                  wayland
                  libGL
                  libx11
                  libxcursor
                  libxi
                  libxrandr
                ]
              );
            in
            rustPlatform.buildRustPackage rec {
              pname = "melba";
              version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
              src = self;

              # The lock file contains git dependencies. fetchCargoVendor includes
              # them in the fixed-output dependency tree, unlike cargoLock alone.
              cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
                pname = "melba";
                version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
                src = self;
                hash = "sha256-KS7C+WdFMmP/kukGxT7IauNFQvxKn/EXnHJUxxYpixU=";
              };
              nativeBuildInputs =
                with pkgs;
                [
                  pkg-config
                ]
                ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ makeWrapper ]
                ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
                  rcodesign
                  icnsify
                ];
              # The command-alias regression starts its own bus, isolated from
              # the listener's desktop and running application.
              nativeCheckInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.dbus ];
              buildInputs =
                pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
                  with pkgs;
                  [
                    alsa-lib
                    dbus
                    libpulseaudio
                    # winit/glutin link GL and X11 at build time even
                    # though the app also dlopens them at run time.
                    libGL
                    libx11
                  ]
                )
                ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.apple-sdk ];

              # Nix's Rust check hook deliberately points SSL_CERT_FILE at a
              # missing path. The proxy regression test constructs a
              # TLS connector before asserting its CONNECT request, so give the
              # test an explicit, sandboxed trust store.
              preCheck = ''
                export SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
              '';

              # The GUI dlopens its Wayland, X11 and GL libraries at run time.
              postFixup =
                pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                  wrapProgram $out/bin/melba \
                    --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}
                ''
                + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
                  rcodesign sign "$out/Applications/Melba.app"
                '';

              postInstall =
                pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                  install -Dm644 packaging/melba.desktop \
                    $out/share/applications/melba.desktop
                  install -Dm644 packaging/icons/melba.svg \
                    $out/share/icons/hicolor/scalable/apps/melba.svg
                  install -Dm644 contrib/omarchy/melba.json.tpl \
                    $out/share/melba/omarchy/melba.json.tpl
                  install -Dm755 contrib/omarchy/melba-theme \
                    $out/share/melba/omarchy/melba-theme
                ''
                + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
                  app="$out/Applications/Melba.app/Contents"
                  mkdir -p "$app/MacOS" "$app/Resources"
                  executable=Melba
                  identifier=rocks.melba.melba
                  cp "$out/bin/melba" "$app/MacOS/$executable"
                  icnsify packaging/macos/icon-1024.png -o "$app/Resources/melba.icns"
                  substitute packaging/macos/Info.plist "$app/Info.plist" \
                    --replace-fail __VERSION__ "${version}" \
                    --replace-fail __BUILD__ "${pkgs.lib.head (pkgs.lib.splitString "-" version)}" \
                    --replace-fail __EXECUTABLE__ "$executable" \
                    --replace-fail __IDENTIFIER__ "$identifier"
                '';

              meta = {
                description = "Fast native Apple Music client for Linux";
                homepage = "https://melba.rocks";
                license = pkgs.lib.licenses.mit;
                mainProgram = "melba";
              };
            };

        in
        {
          default = melba;
          inherit melba;
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin {
          melba-app = melba;
        }
      );

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
