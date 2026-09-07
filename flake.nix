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

  outputs = inputs @ {
    self,
    nixpkgs,
    fenix,
    crane,
    ...
  }: let
    supportedSystems = [
      "aarch64-darwin"
      "aarch64-linux"
      "x86_64-linux"
    ];

    jamyeServerModule = import ./nix/module.nix {inherit nixpkgs self;};

    perSystem = system: let
      pkgs = nixpkgs.legacyPackages.${system};
      lib = pkgs.lib;

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
        filter = path: type: let
          relativePath = pkgs.lib.removePrefix "${toString self}/" (toString path);
          allowedDirectories = [
            "src"
            "tests"
            "migrations"
            "contracts"
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
          type
          != "symlink"
          && pkgs.lib.cleanSourceFilter path type
          && !(blockedNamedSegment || hiddenSegment || environmentSegment)
          && (
            relativePath
            == ""
            || pkgs.lib.elem relativePath requiredRootFiles
            || pkgs.lib.any isAllowedDirectory allowedDirectories
          );
      };
      src = projectSrc;

      commonArgs = {
        inherit src;
        strictDeps = true;
        nativeBuildInputs = [pkgs.pkg-config];
        buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [pkgs.libiconv];
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
        buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [pkgs.libiconv];
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

      moduleEvaluationSystem =
        if pkgs.stdenv.hostPlatform.isLinux
        then system
        else "aarch64-linux";
      moduleEvaluation = nixpkgs.lib.nixosSystem {
        system = moduleEvaluationSystem;
        modules = [
          jamyeServerModule
          {
            system.stateVersion = "25.11";
            services.jamye-server = {
              enable = true;
              environmentFile = "/run/secrets/jamye-server.env";
              objectStorage = {
                publicEndpoint = "https://media.example.test";
                mediaCredentialsFile = "/run/secrets/jamye-server-media.env";
                cleanupCredentialsFile = "/run/secrets/jamye-server-cleanup.env";
                rootCredentialsFile = "/run/secrets/jamye-server-minio-root.env";
              };
            };
          }
        ];
      };
      moduleConfig = moduleEvaluation.config;
      moduleChecks = [
        {
          condition = moduleConfig.services.postgresql.enable;
          message = "PostgreSQL is not enabled by services.jamye-server";
        }
        {
          condition = moduleConfig.services.redis.servers.jamye-server.enable;
          message = "Redis is not enabled by services.jamye-server";
        }
        {
          condition = moduleConfig.services.minio.enable;
          message = "MinIO is not enabled by services.jamye-server";
        }
        {
          condition = lib.all (name: builtins.hasAttr name moduleConfig.systemd.services) [
            "jamye-server-minio-identities"
            "jamye-server-migrate"
            "jamye-server-api"
            "jamye-server-worker"
          ];
          message = "one or more jamye-server systemd services are missing";
        }
        {
          condition =
            moduleConfig.services.jamye-server.environmentFile
            != null
            && moduleConfig.services.jamye-server.objectStorage.publicEndpoint != null
            && moduleConfig.services.jamye-server.objectStorage.mediaCredentialsFile != null
            && moduleConfig.services.jamye-server.objectStorage.cleanupCredentialsFile != null
            && moduleConfig.services.jamye-server.objectStorage.rootCredentialsFile != null;
          message = "the evaluated module lost required deployment inputs";
        }
        {
          condition =
            lib.elem "jamye-server-migrate.service" moduleConfig.systemd.services.jamye-server-api.requires
            && lib.elem "jamye-server-minio-identities.service" moduleConfig.systemd.services.jamye-server-api.requires
            && lib.elem "redis-jamye-server.service" moduleConfig.systemd.services.jamye-server-api.requires;
          message = "the API is not ordered after every required stack dependency";
        }
        {
          condition =
            !(lib.elem "/run/secrets/jamye-server-minio-root.env" moduleConfig.systemd.services.jamye-server-api.serviceConfig.EnvironmentFile)
            && !(lib.elem "/run/secrets/jamye-server-minio-root.env" moduleConfig.systemd.services.jamye-server-worker.serviceConfig.EnvironmentFile);
          message = "MinIO root credentials reached an application service";
        }
      ];
      failedModuleChecks = builtins.filter (check: !check.condition) moduleChecks;
      nixosModuleCheck =
        if failedModuleChecks != []
        then
          throw (
            "jamye-server NixOS module evaluation failed: "
            + lib.concatMapStringsSep "; " (check: check.message) failedModuleChecks
          )
        else
          pkgs.runCommand "jamye-server-nixos-module-evaluation" {} ''
            mkdir -p "$out"
            printf '%s\n' ${lib.escapeShellArg moduleEvaluationSystem} > "$out/evaluated-system"
          '';
    in {
      packages = {
        inherit api worker;
      };

      checks =
        {
          inherit api worker;
          "contract-drift" = contractDrift;

          cargo-fmt = craneLib.cargoFmt {inherit src;};

          cargo-clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets --all-features -- --deny warnings";
            }
          );

          "cargo-test-default" = cargoTestDefault;

          "cargo-test-all-features" = cargoTestAllFeatures;

          "nixos-module" = nixosModuleCheck;

          architecture = craneLib.cargoTest (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoTestExtraArgs = "--test architecture";
            }
          );
        }
        // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          "nixos-module-smoke" = import ./nix/smoke-test.nix {
            inherit jamyeServerModule pkgs;
          };
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
  in {
    packages = nixpkgs.lib.mapAttrs (_: value: value.packages) systemOutputs;
    checks = nixpkgs.lib.mapAttrs (_: value: value.checks) systemOutputs;
    devShells = nixpkgs.lib.mapAttrs (_: value: value.devShells) systemOutputs;
    nixosModules.default = jamyeServerModule;
  };
}
