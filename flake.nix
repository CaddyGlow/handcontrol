{
  description = "Nix flake for the HandControl project";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config = {
            allowUnfree = true;
            android_sdk.accept_license = true;
          };
        };
        lib = pkgs.lib;
        isLinux = pkgs.stdenv.isLinux;
        isDarwin = pkgs.stdenv.isDarwin;
        androidPackages =
          if isLinux then
            pkgs.androidenv.composeAndroidPackages {
              platformVersions = [ "36" ];
              buildToolsVersions = [ "36.0.0" ];
              includeNDK = true;
              includeEmulator = false;
              ndkVersions = [ "27.0.12077973" ];
            }
          else
            null;
        javaToolchain = pkgs.openjdk17;
        cargoToml = lib.importTOML ./Cargo.toml;
        crateName = cargoToml.package.name;
        crateVersion = cargoToml.package.version;

        projectDescription = "HandControl secure remote control server";

        cratePackage = pkgs.rustPlatform.buildRustPackage {
          pname = crateName;
          version = crateVersion;
          src = lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          cargoHash = lib.fakeSha256;
          inherit nativeBuildInputs;
          buildInputs = [ ];
          meta = with lib; {
            description = projectDescription;
            license = licenses.mit;
            maintainers = [ ];
          };
        };

        # Cross-compilation helper function
        mkCrossPackage =
          crossPkgs: targetName:
          crossPkgs.rustPlatform.buildRustPackage {
            pname = "${crateName}-${targetName}";
            version = crateVersion;
            src = lib.cleanSource ./.;
            cargoLock.lockFile = ./Cargo.lock;
            cargoHash = lib.fakeSha256;

            # Don't include pkg-config for cross-compilation as it often fails
            # and isn't needed for static Rust binaries
            nativeBuildInputs = [ ];
            buildInputs = [ ];

            meta = with lib; {
              description = "HandControl secure remote control server (${targetName})";
              license = licenses.mit;
              maintainers = [ ];
            };
          };

        # Android build helper function using Nix cross-compilation
        mkAndroidPackage =
          androidPkgs: targetName:
          androidPkgs.rustPlatform.buildRustPackage {
            pname = "${crateName}-android-${targetName}";
            version = crateVersion;
            src = lib.cleanSource ./.;
            cargoLock.lockFile = ./Cargo.lock;
            cargoHash = lib.fakeSha256;

            nativeBuildInputs = [ ];
            buildInputs = [ ];

            # Disable tests for cross-compilation
            doCheck = false;

            meta = with lib; {
              description = "HandControl secure remote control server (Android ${targetName})";
              license = licenses.mit;
              maintainers = [ ];
            };
          };

        androidEmulator =
          if isLinux then
            pkgs.androidenv.emulateApp {
              name = "Pixel 10";
              platformVersion = "36";
              systemImageType = "google_apis_playstore";
              abiVersion = "x86_64";
              configOptions = {
                # https://android.googlesource.com/platform/external/qemu/+/refs/heads/master/android/avd/hardware-properties.ini
                "hw.device.manufacturer" = "Google";
                "hw.device.name" = "pixel_10";
                "hw.ramSize" = "8192";
                "hw.lcd.width" = "1080";
                "hw.lcd.height" = "2424";
                "hw.lcd.density" = "460";
                "hw.keyboard" = "yes";
              };
            }
          else
            null;
        nativeBuildInputs =
          [ pkgs.pkg-config ]
          ++ lib.optionals isLinux [ androidEmulator ];
        commonDevPackages = [
          pkgs.rustc
          pkgs.cargo
          pkgs.clippy
          pkgs.rustfmt
          pkgs.rust-analyzer
          pkgs.cargo-edit
          pkgs.cargo-deny
          pkgs.cargo-audit
          pkgs.cargo-ndk
          pkgs.rustup
          pkgs.pkg-config
          pkgs.protobuf
          pkgs.openssl
          javaToolchain
          pkgs.gradle
        ];
        linuxDevPackages =
          if isLinux then
            [
              pkgs.cargo-tarpaulin
              androidPackages.androidsdk
              pkgs.androidStudioPackages.dev
            ]
          else
            [];
        darwinDevPackages =
          if isDarwin then
            [ pkgs.libiconv ]
          else
            [];

      in
      {
        packages = {
          default = cratePackage;

          # Cross-platform builds
          # Windows
          windows-x86_64 = mkCrossPackage pkgs.pkgsCross.mingwW64 "windows-x86_64";

          # macOS
          macos-aarch64 = mkCrossPackage pkgs.pkgsCross.aarch64-darwin "macos-aarch64";
          macos-x86_64 = mkCrossPackage pkgs.pkgsCross.x86_64-darwin "macos-x86_64";

          # Linux
          linux-x86_64 = mkCrossPackage pkgs.pkgsCross.gnu64 "linux-x86_64";
          linux-aarch64 = mkCrossPackage pkgs.pkgsCross.aarch64-multiplatform "linux-aarch64";

          # Android builds for common architectures
          android-aarch64 = mkAndroidPackage pkgs.pkgsCross.aarch64-android-prebuilt "aarch64";
          android-armv7 = mkAndroidPackage pkgs.pkgsCross.armv7a-android-prebuilt "armv7";
          android-x86_64 = mkAndroidPackage pkgs.pkgsCross.x86_64-android-prebuilt "x86_64";
        };

        apps.default = {
          type = "app";
          program = "${cratePackage}/bin/${crateName}";
        };

        devShells.default = pkgs.mkShell {
          packages = commonDevPackages ++ linuxDevPackages ++ darwinDevPackages;

          inherit nativeBuildInputs;

          shellHook =
            ''
              export JAVA_HOME=${javaToolchain}
            ''
            + lib.optionalString isLinux ''
              export ANDROID_HOME=${androidPackages.androidsdk}
              export ANDROID_SDK_ROOT=${androidPackages.androidsdk}
              export QT_QPA_PLATFORM=xcb
              export NIX_ANDROID_EMULATOR_FLAGS="-no-snapshot -gpu swiftshader_indirect"
              if [ -d "${androidPackages.androidsdk}/ndk" ]; then
                export ANDROID_NDK_HOME=$(ls -d ${androidPackages.androidsdk}/ndk/* | head -n1)
                export ANDROID_NDK_ROOT=$ANDROID_NDK_HOME
              fi
            '';
        };

        formatter = pkgs.alejandra;

        checks.build = cratePackage;
      }
    );
}
