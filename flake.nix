{
  description = "jamye-server Rust/Axum server";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    crane.url = "github:ipetkov/crane";
  };

  outputs =
    inputs@{
      self,
      nixpkgs,
      fenix,
      crane,
      ...
    }:
    let
      supportedSystems = [
        "aarch64-darwin"
        "x86_64-linux"
      ];

      perSystem =
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};

          # rust-toolchain.toml is the only declaration of the Rust release,
          # profile, components, and compilation targets. Do not duplicate any
          # of those values in this file.
          rustToolchain = fenix.packages.${system}.fromToolchainFile {
            file = ./rust-toolchain.toml;
            # Integrity of the official manifest selected by the toolchain
            # file. This is not a second Rust version declaration.
            sha256 = "sha256-P30Tm3O7vQAE725YtDCDHGjNrSsfZO4us11UwJGZSJo=";
          };
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
          src = craneLib.cleanCargoSource self;

          commonArgs = {
            inherit src;
            strictDeps = true;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
          };

          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          api = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoExtraArgs = "--locked --bin api";
              doCheck = false;
              meta.mainProgram = "api";
            }
          );

          worker = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoExtraArgs = "--locked --bin worker";
              doCheck = false;
              meta.mainProgram = "worker";
            }
          );
        in
        {
          packages = {
            inherit api worker;
            default = api;
          };

          checks = {
            inherit api worker;

            cargo-fmt = craneLib.cargoFmt { inherit src; };

            cargo-clippy = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoClippyExtraArgs = "--locked --all-targets --all-features -- --deny warnings";
              }
            );

            cargo-test-default = craneLib.cargoTest (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoTestExtraArgs = "--locked --all-targets";
              }
            );

            cargo-test-all-features = craneLib.cargoTest (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoTestExtraArgs = "--locked --all-targets --all-features";
              }
            );

            architecture = craneLib.cargoTest (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoTestExtraArgs = "--locked --test architecture";
              }
            );
          };

          devShells.default = pkgs.mkShell {
            packages = [
              rustToolchain
              pkgs.cargo-deny
              pkgs.coreutils
              pkgs.curl
              pkgs.git
              pkgs.gitleaks
              pkgs.jq
              pkgs.just
              pkgs.minio-client
              pkgs.podman
              pkgs.podman-compose
              pkgs.sqlx-cli
            ];

            # podman compose must use the provider pinned by flake.lock. An
            # ambient Docker Compose or Homebrew provider is never selected.
            PODMAN_COMPOSE_PROVIDER = "${pkgs.podman-compose}/bin/podman-compose";
            RUST_BACKTRACE = "1";
          };
        };

      systemOutputs = nixpkgs.lib.genAttrs supportedSystems perSystem;
    in
    {
      packages = nixpkgs.lib.mapAttrs (_: value: value.packages) systemOutputs;
      checks = nixpkgs.lib.mapAttrs (_: value: value.checks) systemOutputs;
      devShells = nixpkgs.lib.mapAttrs (_: value: value.devShells) systemOutputs;
    };
}
