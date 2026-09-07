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
        "aarch64-linux"
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
          projectSrc = pkgs.lib.cleanSourceWith {
            # Crane's default source filter retains Rust/Cargo sources only,
            # but this repository's compiled contract generation and tests
            # also consume the listed contract, migration, documentation, and
            # task-script inputs. Keep that exact build input set while
            # excluding local build products, agent state, and dotfiles.
            src = self;
            filter = path: type:
              let
                relativePath = pkgs.lib.removePrefix "${toString self}/" (toString path);
                allowedDirectories = [
                  "src"
                  "tests"
                  "migrations"
                  "contracts"
                  "data"
                  "production_composition"
                  "scripts"
                  "docs/adr"
                ];
                requiredRootFiles = [
                  "Cargo.lock"
                  "Cargo.toml"
                  "rust-toolchain.toml"
                ];
                isAllowedDirectory = directory:
                  relativePath == directory || pkgs.lib.hasPrefix "${directory}/" relativePath;
                blockedNamedSegment = (builtins.match "(^|.*/)(target|result|[.]git|[.]agents)(/.*|$)" relativePath) != null;
                hiddenSegment = (builtins.match "(^|.*/)[.][^/]+(/.*|$)" relativePath) != null;
                environmentSegment = (builtins.match "(^|.*/)[.]env[^/]*(/.*|$)" relativePath) != null;
              in
              type != "symlink"
              && pkgs.lib.cleanSourceFilter path type
              && !(blockedNamedSegment || hiddenSegment || environmentSegment)
              && (
                relativePath == ""
                || pkgs.lib.elem relativePath requiredRootFiles
                || pkgs.lib.any isAllowedDirectory allowedDirectories
              );
          };
          src = projectSrc;

          commonArgs = {
            inherit src;
            strictDeps = true;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
          };

          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          contractDrift = craneLib.mkCargoDerivation {
            inherit cargoArtifacts;
            src = projectSrc;
            strictDeps = true;
            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.coreutils
              pkgs.diffutils
              pkgs.findutils
              pkgs.gnugrep
            ];
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
            pnameSuffix = "-contract-drift";

            # Reuse the repository contract checker instead of reimplementing
            # its provenance and byte-comparison logic in Nix.
            # Crane supplies a vendored, offline Cargo environment and the
            # reusable dependency artifacts inside the sandbox.
            buildPhaseCargoCommand = ''
              IN_NIX_SHELL=1 CARGO_NET_OFFLINE=true \
                bash ./scripts/check-contracts.sh
            '';
            installPhaseCommand = "mkdir -p $out";
          };

          cargoTestDefault = craneLib.mkCargoDerivation (
            commonArgs
            // {
              inherit cargoArtifacts;
              pnameSuffix = "-test-default";
              # Compile the complete default-feature test inventory without
              # running its service-backed tests inside the Nix sandbox.
              buildPhaseCargoCommand = "cargo test --locked --all-targets --no-run";
              installPhaseCommand = "mkdir -p $out";
            }
          );

          cargoTestAllFeatures = craneLib.mkCargoDerivation (
            commonArgs
            // {
              inherit cargoArtifacts;
              pnameSuffix = "-test-all-features";
              # Compile the complete all-feature test inventory without
              # running its service-backed tests inside the Nix sandbox.
              buildPhaseCargoCommand = "cargo test --locked --all-targets --all-features --no-run";
              installPhaseCommand = "mkdir -p $out";
            }
          );

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
          };

          checks = {
            inherit api worker;
            "contract-drift" = contractDrift;

            cargo-fmt = craneLib.cargoFmt { inherit src; };

            cargo-clippy = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoClippyExtraArgs = "--all-targets --all-features -- --deny warnings";
              }
            );

            "cargo-test-default" = cargoTestDefault;

            "cargo-test-all-features" = cargoTestAllFeatures;

            architecture = craneLib.cargoTest (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoTestExtraArgs = "--test architecture";
              }
            );
          };

          devShells.default = pkgs.mkShell {
            packages = [
              rustToolchain
              pkgs.cargo-deny
              pkgs.cargo-llvm-cov
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
      nixosModules.default = {
        options = { };
        config = { };
      };
    };
}
