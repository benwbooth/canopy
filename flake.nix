{
  description = "canopy: Google Drive sync for Linux, like Insync (Rust + Qt Quick)";

  # The same nixpkgs revision as grove, so canopy uses the Qt build the
  # desktop's style and platform plugins were made for.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/ab5c18fdedfc8f9bbc9d773f10a540790e2787a7";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      qtFor = pkgs: with pkgs.qt6; [ qtbase qtdeclarative qtsvg qtwayland qtimageformats ];
      qtEnvFor = pkgs: pkgs.qt6.env "qt-canopy-${pkgs.qt6.qtbase.version}" ((builtins.tail (qtFor pkgs)) ++ [ pkgs.libGL ]);
      # Nothing else to link: TLS is rustls, SQLite is built in, D-Bus is zbus.
      linkedFor = pkgs: [ ];
    in
    {
      packages = forAll (pkgs:
        let
          qtEnv = qtEnvFor pkgs;
          cargoDeps = pkgs.rustPlatform.importCargoLock { lockFile = ./Cargo.lock; };
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type: !(pkgs.lib.hasInfix "/target" path) && baseNameOf path != ".qt-env" && baseNameOf path != "result";
          };
          # Build in a persistent target directory (/var/tmp/cargo-nix, exposed
          # to the sandbox by the system flake) so rebuilds only recompile what
          # changed. Falls back to a throwaway directory when it isn't there.
          # kache stays out of these builds: CARGO_HOME is the build's own.
          persistentCargo = name: cargoArgs: ''
            runHook preBuild
            workDir=$TMPDIR/cargo-nix
            if [ -w /var/tmp/cargo-nix ]; then
              workDir=/var/tmp/cargo-nix/${name}
            fi
            # The store vendor copy only changes with Cargo.lock; the per-build
            # copy gets new mtimes, which would rebuild every dependency.
            sed -i "s|^directory = .*|directory = \"${cargoDeps}\"|" "$NIX_BUILD_TOP/.cargo/config.toml"
            (
              # Writable by everyone: each build may run as a different nixbld
              # user. Kept to this subshell so the outputs get normal modes.
              umask 000
              mkdir -p "$workDir/src" "$workDir/target"
              touch "$workDir"
              # Only rewrite files whose content changed (cargo checks mtimes).
              rsync -rc --no-times --no-perms --chmod=a+rwX --delete ./ "$workDir/src/"
              # The linker wrapper rejects output paths outside the build dir.
              export NIX_ENFORCE_PURITY=0
              cd "$workDir/src"
              # cxx-qt copies its QML exports with fs::copy, which also
              # chmods the destination: that fails on a file left by another
              # nixbld user. The exports are regenerated every build anyway.
              rm -rf "$workDir/target/cxxqt"
              CARGO_HOME="$NIX_BUILD_TOP/.cargo" CARGO_TARGET_DIR="$workDir/target" \
                cargo build ${cargoArgs} --offline --locked -j "$NIX_BUILD_CORES"
              find "$workDir/target" -mindepth 1 -user "$(id -u)" ! -perm -o+w -exec chmod a+rwX {} + 2>/dev/null || true
            )
            ln -sfn "$workDir/target" target
            runHook postBuild
          '';
          # WhiteSur's themes alone, as the window's fallback icon themes.
          # The package's own share/icons also has hicolor, an empty
          # skeleton with no icon-theme.cache: Qt checks every one of its
          # ~5000 size folders for each icon name it can't find.
          whitesurIcons = pkgs.runCommand "canopy-whitesur-icons" { } ''
            mkdir -p $out/share/icons
            for t in WhiteSur WhiteSur-dark WhiteSur-light; do
              ln -s ${pkgs.whitesur-icon-theme}/share/icons/$t $out/share/icons/$t
            done
          '';
          canopy = pkgs.rustPlatform.buildRustPackage {
            pname = "canopy";
            version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
            inherit src cargoDeps;
            nativeBuildInputs = [ pkgs.pkg-config pkgs.qt6.wrapQtAppsHook pkgs.qt6.qtbase pkgs.rsync pkgs.makeWrapper ];
            buildInputs = qtFor pkgs ++ [ qtEnv pkgs.libGL ] ++ linkedFor pkgs;
            # cxx-qt wants every Qt module under one prefix.
            env.QMAKE = "${qtEnv}/bin/qmake";
            preBuild = ''
              # (Qt's setup hook sets QMAKE to qtbase's alone.)
              export QMAKE=${qtEnv}/bin/qmake
            '';
            buildPhase = persistentCargo "canopy" "--release -p canopy -p canopy-core";
            installPhase = ''
              runHook preInstall
              install -Dm755 target/release/canopy $out/bin/canopy
              install -Dm755 target/release/canopyd $out/bin/canopyd
              install -Dm644 icons/canopy.svg $out/share/icons/hicolor/scalable/apps/canopy.svg
              for f in icons/tray/*.svg; do install -Dm644 "$f" $out/share/icons/hicolor/scalable/status/$(basename "$f"); done
              # A cache, so names not here cost a lookup, not a file check
              # in every hicolor size folder (the window searches $out/share).
              ${pkgs.gtk3}/bin/gtk-update-icon-cache -f -t $out/share/icons/hicolor
              install -Dm644 data/canopy.desktop $out/share/applications/canopy.desktop
              install -Dm644 data/canopy-open.desktop $out/share/applications/canopy-open.desktop
              install -Dm644 data/canopy-mime.xml $out/share/mime/packages/canopy.xml
              install -Dm644 data/canopyd.service $out/lib/systemd/user/canopyd.service
              substituteInPlace $out/lib/systemd/user/canopyd.service --replace-fail /usr/bin/canopyd $out/bin/canopyd
              runHook postInstall
            '';
            doCheck = false;
            dontWrapQtApps = true;
            postFixup = ''
              wrapQtApp $out/bin/canopy \
                --set-default CANOPY_FONT_DIR ${pkgs.inter}/share/fonts/truetype \
                --suffix XDG_DATA_DIRS : ${whitesurIcons}/share
            '';
            meta = {
              description = "Google Drive sync for Linux, like Insync";
              homepage = "https://github.com/benwbooth/canopy";
              license = pkgs.lib.licenses.mit;
              mainProgram = "canopy";
              platforms = pkgs.lib.platforms.linux;
            };
          };
        in
        { default = canopy; inherit canopy; });

      apps = forAll (pkgs: {
        default = { type = "app"; program = "${self.packages.${pkgs.system}.default}/bin/canopy"; };
      });

      # `nix develop`: Rust and the same Qt; plain `cargo` then uses the
      # host's kache setup (~/.cargo/config.toml).
      devShells = forAll (pkgs:
        let qtEnv = qtEnvFor pkgs; in
        {
          default = pkgs.mkShell {
            packages = [ pkgs.cargo pkgs.rustc pkgs.clippy pkgs.rustfmt pkgs.pkg-config qtEnv pkgs.libGL pkgs.git pkgs.imagemagick pkgs.xvfb-run pkgs.sqlite ];
            buildInputs = linkedFor pkgs;
            QMAKE = "${qtEnv}/bin/qmake";
            # Link with this shell's compiler (a system clang links against the
            # system glibc and the binary can't load this nixpkgs' Qt).
            "CARGO_TARGET_${pkgs.stdenv.hostPlatform.rust.cargoEnvVarTarget}_LINKER" = "cc";
            shellHook = ''
              # Only in canopy's own checkout (never in the folder a shell
              # happened to start in).
              if [ -f Cargo.toml ] && grep -q '^name = "canopy"' Cargo.toml; then
                ln -sfn ${qtEnv} .qt-env
              fi
              export NIX_LDFLAGS="''${NIX_LDFLAGS//"-rpath $out/lib"/}"
              export CANOPY_FONT_DIR=''${CANOPY_FONT_DIR:-${pkgs.inter}/share/fonts/truetype}
              export XDG_DATA_DIRS="${pkgs.whitesur-icon-theme}/share:''${XDG_DATA_DIRS:-}"
            '';
          };
        });
    };
}
