{
  nixpkgs,
  self,
}: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.jamye-server;
  system = pkgs.stdenv.hostPlatform.system;
  apiPackage = self.packages.${system}.api;
  workerPackage = self.packages.${system}.worker;
  minioPkgs = import nixpkgs {
    inherit system;
    config.allowInsecurePredicate = package: lib.getName package == "minio";
  };
  minioPackage = minioPkgs.minio;

  serviceName = "jamye-server";
  databaseName = serviceName;
  databaseUrl = "postgresql://${serviceName}@localhost/${databaseName}?host=/run/postgresql";
  redisUnit = "redis-${serviceName}.service";
  redisUrl = "redis://127.0.0.1:6379";
  minioEndpoint = "http://127.0.0.1:${toString cfg.objectStorage.port}";
  minioHealthUrl = "${minioEndpoint}/minio/health/live";

  publicObjectStorageEndpoint =
    if cfg.objectStorage.publicEndpoint == null
    then ""
    else cfg.objectStorage.publicEndpoint;

  runtimeExports = ''
    export JAMYE_ENVIRONMENT=production
    export JAMYE_LISTEN_ADDR=${lib.escapeShellArg cfg.listenAddress}
    export DATABASE_URL=${lib.escapeShellArg databaseUrl}
    export REDIS_URL=${lib.escapeShellArg redisUrl}
    export JAMYE_MINIO_HEALTH_URL=${lib.escapeShellArg minioHealthUrl}
    export JAMYE_OBJECT_STORAGE_ENDPOINT=${lib.escapeShellArg minioEndpoint}
    export JAMYE_OBJECT_STORAGE_PUBLIC_ENDPOINT=${lib.escapeShellArg publicObjectStorageEndpoint}
    export JAMYE_OBJECT_STORAGE_REGION=${lib.escapeShellArg cfg.objectStorage.region}
    export JAMYE_OBJECT_STORAGE_BUCKET=${lib.escapeShellArg cfg.objectStorage.bucket}
  '';

  startApi = pkgs.writeShellScript "jamye-server-api-start" ''
    set -euo pipefail
    ${runtimeExports}
    exec ${apiPackage}/bin/api
  '';

  startWorker = pkgs.writeShellScript "jamye-server-worker-start" ''
    set -euo pipefail
    ${runtimeExports}
    exec ${workerPackage}/bin/worker
  '';

  startMigration = pkgs.writeShellScript "jamye-server-migrate-start" ''
    set -euo pipefail
    export DATABASE_URL=${lib.escapeShellArg databaseUrl}
    exec ${pkgs.sqlx-cli}/bin/sqlx migrate run --source ${../migrations}
  '';

  mediaPolicy = pkgs.writeText "jamye-server-minio-media-policy.json" (
    builtins.toJSON {
      Version = "2012-10-17";
      Statement = [
        {
          Sid = "JamyeServerBucketLifecycle";
          Effect = "Allow";
          Action = [
            "s3:CreateBucket"
            "s3:GetBucketLocation"
            "s3:ListBucket"
          ];
          Resource = ["arn:aws:s3:::${cfg.objectStorage.bucket}"];
        }
        {
          Sid = "JamyeServerPrivateMedia";
          Effect = "Allow";
          Action = [
            "s3:GetObject"
            "s3:PutObject"
          ];
          Resource = ["arn:aws:s3:::${cfg.objectStorage.bucket}/*"];
        }
      ];
    }
  );

  cleanupPolicy = pkgs.writeText "jamye-server-minio-cleanup-policy.json" (
    builtins.toJSON {
      Version = "2012-10-17";
      Statement = [
        {
          Sid = "JamyeServerAccountObjectDeletion";
          Effect = "Allow";
          Action = ["s3:DeleteObject"];
          Resource = ["arn:aws:s3:::${cfg.objectStorage.bucket}/*"];
        }
      ];
    }
  );

  baseEnvironmentFiles = lib.optional (cfg.environmentFile != null) cfg.environmentFile;
  mediaEnvironmentFiles =
    lib.optional (
      cfg.objectStorage.mediaCredentialsFile != null
    )
    cfg.objectStorage.mediaCredentialsFile;
  cleanupEnvironmentFiles =
    lib.optional (
      cfg.objectStorage.cleanupCredentialsFile != null
    )
    cfg.objectStorage.cleanupCredentialsFile;
  rootEnvironmentFiles =
    lib.optional (
      cfg.objectStorage.rootCredentialsFile != null
    )
    cfg.objectStorage.rootCredentialsFile;

  commonHardening = {
    NoNewPrivileges = true;
    PrivateTmp = true;
    ProtectHome = true;
    ProtectSystem = "strict";
    LockPersonality = true;
    RestrictSUIDSGID = true;
    UMask = "0077";
  };
in {
  options.services.jamye-server = {
    enable = lib.mkEnableOption "the complete jamye-server application stack";

    listenAddress = lib.mkOption {
      type = lib.types.str;
      default = "0.0.0.0:8080";
      description = "Address used by the jamye-server API. The module does not open the firewall.";
    };

    environmentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      example = ''config.sops.templates."jamye-server.env".path'';
      description = ''
        Runtime EnvironmentFile containing authentication, OAuth, and optional
        Expo credentials. Infrastructure addresses are supplied authoritatively
        by this module after the file is loaded.
      '';
    };

    objectStorage = {
      publicEndpoint = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "https://media.example.com";
        description = ''
          Browser-reachable HTTPS origin for path-style MinIO presigned URLs.
          The ingress and TLS endpoint remain deployment-owned.
        '';
      };

      bucket = lib.mkOption {
        type = lib.types.str;
        default = "jamye-server-media";
        description = "Private bucket managed at API startup by the media identity.";
      };

      region = lib.mkOption {
        type = lib.types.str;
        default = "us-east-1";
        description = "S3 signing region used by MinIO and jamye-server.";
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 9000;
        description = "MinIO S3 port exposed on the host for the deployment-owned ingress.";
      };

      consolePort = lib.mkOption {
        type = lib.types.port;
        default = 9001;
        description = "Loopback-only MinIO console port. The browser UI is disabled.";
      };

      mediaCredentialsFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        example = ''config.sops.templates."jamye-server-media.env".path'';
        description = ''
          EnvironmentFile containing JAMYE_OBJECT_STORAGE_ACCESS_KEY_ID and
          JAMYE_OBJECT_STORAGE_SECRET_ACCESS_KEY for the media identity.
        '';
      };

      cleanupCredentialsFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        example = ''config.sops.templates."jamye-server-cleanup.env".path'';
        description = ''
          EnvironmentFile containing JAMYE_ACCOUNT_OBJECT_DELETION_ACCESS_KEY_ID
          and JAMYE_ACCOUNT_OBJECT_DELETION_SECRET_ACCESS_KEY for the deletion-only identity.
        '';
      };

      rootCredentialsFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        example = ''config.sops.templates."jamye-server-minio-root.env".path'';
        description = ''
          EnvironmentFile containing only MINIO_ROOT_USER and
          MINIO_ROOT_PASSWORD. It is never loaded by the API or worker.
        '';
      };
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.environmentFile != null;
        message = "services.jamye-server.environmentFile must be set for production authentication configuration.";
      }
      {
        assertion = cfg.objectStorage.mediaCredentialsFile != null;
        message = "services.jamye-server.objectStorage.mediaCredentialsFile must be set for the dedicated media identity.";
      }
      {
        assertion = cfg.objectStorage.cleanupCredentialsFile != null;
        message = "services.jamye-server.objectStorage.cleanupCredentialsFile must be set for the dedicated cleanup identity.";
      }
      {
        assertion = cfg.objectStorage.rootCredentialsFile != null;
        message = "services.jamye-server.objectStorage.rootCredentialsFile must be set so MinIO never starts with default credentials.";
      }
      {
        assertion =
          cfg.objectStorage.publicEndpoint
          != null
          && lib.hasPrefix "https://" cfg.objectStorage.publicEndpoint
          && builtins.match ".*(localhost|127[.]0[.]0[.]1).*" cfg.objectStorage.publicEndpoint == null;
        message = "services.jamye-server.objectStorage.publicEndpoint must be an external HTTPS origin.";
      }
      {
        assertion = let
          files = [
            cfg.environmentFile
            cfg.objectStorage.mediaCredentialsFile
            cfg.objectStorage.cleanupCredentialsFile
            cfg.objectStorage.rootCredentialsFile
          ];
          configuredFiles = builtins.filter (file: file != null) files;
        in
          builtins.length configuredFiles == builtins.length (lib.unique configuredFiles);
        message = "services.jamye-server secret EnvironmentFiles must be distinct so MinIO root credentials cannot reach application processes.";
      }
      {
        assertion = cfg.objectStorage.port != cfg.objectStorage.consolePort;
        message = "services.jamye-server MinIO S3 and console ports must be distinct.";
      }
    ];

    users.groups.${serviceName} = {};
    users.users.${serviceName} = {
      isSystemUser = true;
      group = serviceName;
      home = "/var/lib/${serviceName}";
    };

    systemd.tmpfiles.rules = ["d /var/lib/${serviceName} 0750 ${serviceName} ${serviceName} -"];

    services.postgresql = {
      enable = true;
      ensureDatabases = [databaseName];
      ensureUsers = [
        {
          name = serviceName;
          ensureDBOwnership = true;
        }
      ];
    };

    services.redis.servers.${serviceName} = {
      enable = true;
      bind = "127.0.0.1";
      port = 6379;
      openFirewall = false;
    };

    services.minio = {
      enable = true;
      # MinIO is abandoned upstream and marked insecure by nixpkgs. Keep the
      # exception scoped to jamye-server's pinned MinIO package set instead of
      # weakening the host's global nixpkgs policy.
      package = minioPackage;
      listenAddress = ":${toString cfg.objectStorage.port}";
      consoleAddress = "127.0.0.1:${toString cfg.objectStorage.consolePort}";
      rootCredentialsFile = cfg.objectStorage.rootCredentialsFile;
      region = cfg.objectStorage.region;
      browser = false;
    };

    systemd.targets.${serviceName} = {
      description = "jamye-server application and required infrastructure";
      wantedBy = ["multi-user.target"];
      requires = [
        "postgresql.target"
        redisUnit
        "minio.service"
        "jamye-server-minio-identities.service"
        "jamye-server-migrate.service"
        "jamye-server-api.service"
        "jamye-server-worker.service"
      ];
      after = [
        "postgresql.target"
        redisUnit
        "minio.service"
        "jamye-server-minio-identities.service"
        "jamye-server-migrate.service"
      ];
    };

    systemd.services.postgresql.partOf = ["${serviceName}.target"];
    systemd.services.${"redis-${serviceName}"}.partOf = ["${serviceName}.target"];
    systemd.services.minio.partOf = ["${serviceName}.target"];

    systemd.services.jamye-server-minio-identities = {
      description = "jamye-server MinIO identities and least-privilege policies";
      after = ["minio.service"];
      requires = ["minio.service"];
      before = [
        "jamye-server-api.service"
        "jamye-server-worker.service"
      ];
      partOf = ["${serviceName}.target"];
      path = [
        pkgs.coreutils
        pkgs.curl
        (lib.getOutput "getent" pkgs.glibc)
        pkgs.minio-client
      ];
      serviceConfig =
        commonHardening
        // {
          Type = "oneshot";
          RemainAfterExit = true;
          DynamicUser = true;
          RuntimeDirectory = "jamye-server-mc";
          RuntimeDirectoryMode = "0700";
          Environment = "MC_CONFIG_DIR=/run/jamye-server-mc";
          EnvironmentFile = mediaEnvironmentFiles ++ cleanupEnvironmentFiles ++ rootEnvironmentFiles;
          TimeoutStartSec = "150s";
        };
      script = ''
        set -euo pipefail

        required_keys=(
          MINIO_ROOT_USER
          MINIO_ROOT_PASSWORD
          JAMYE_OBJECT_STORAGE_ACCESS_KEY_ID
          JAMYE_OBJECT_STORAGE_SECRET_ACCESS_KEY
          JAMYE_ACCOUNT_OBJECT_DELETION_ACCESS_KEY_ID
          JAMYE_ACCOUNT_OBJECT_DELETION_SECRET_ACCESS_KEY
        )
        for key in "''${required_keys[@]}"; do
          if [ -z "''${!key:-}" ]; then
            echo "Missing required credential: $key" >&2
            exit 1
          fi
        done

        if [ "$MINIO_ROOT_USER" = "$JAMYE_OBJECT_STORAGE_ACCESS_KEY_ID" ] \
          || [ "$MINIO_ROOT_USER" = "$JAMYE_ACCOUNT_OBJECT_DELETION_ACCESS_KEY_ID" ] \
          || [ "$MINIO_ROOT_PASSWORD" = "$JAMYE_OBJECT_STORAGE_SECRET_ACCESS_KEY" ] \
          || [ "$MINIO_ROOT_PASSWORD" = "$JAMYE_ACCOUNT_OBJECT_DELETION_SECRET_ACCESS_KEY" ] \
          || [ "$JAMYE_OBJECT_STORAGE_ACCESS_KEY_ID" = "$JAMYE_ACCOUNT_OBJECT_DELETION_ACCESS_KEY_ID" ] \
          || [ "$JAMYE_OBJECT_STORAGE_SECRET_ACCESS_KEY" = "$JAMYE_ACCOUNT_OBJECT_DELETION_SECRET_ACCESS_KEY" ]; then
          echo "MinIO root, media, and cleanup credentials must all be distinct" >&2
          exit 1
        fi

        endpoint=${lib.escapeShellArg minioEndpoint}
        alias_name=jamye_server_bootstrap
        deadline=$((SECONDS + 120))
        attempts=0
        last_stage=not-started

        provision() {
          last_stage=health
          curl -fsS --max-time 5 "$endpoint/minio/health/ready" >/dev/null || return 1
          last_stage=alias
          mc --quiet alias set "$alias_name" "$endpoint" \
            "$MINIO_ROOT_USER" "$MINIO_ROOT_PASSWORD" >/dev/null || return 1
          last_stage=media-user
          printf '%s\n%s\n' \
            "$JAMYE_OBJECT_STORAGE_ACCESS_KEY_ID" \
            "$JAMYE_OBJECT_STORAGE_SECRET_ACCESS_KEY" \
            | mc --quiet admin user add "$alias_name" >/dev/null || return 1
          last_stage=cleanup-user
          printf '%s\n%s\n' \
            "$JAMYE_ACCOUNT_OBJECT_DELETION_ACCESS_KEY_ID" \
            "$JAMYE_ACCOUNT_OBJECT_DELETION_SECRET_ACCESS_KEY" \
            | mc --quiet admin user add "$alias_name" >/dev/null || return 1
          last_stage=media-policy
          mc --quiet admin policy create "$alias_name" jamye-server-media ${mediaPolicy} >/dev/null || return 1
          last_stage=cleanup-policy
          mc --quiet admin policy create "$alias_name" jamye-server-cleanup ${cleanupPolicy} >/dev/null || return 1
          last_stage=media-policy-attach
          mc --quiet admin policy attach "$alias_name" jamye-server-media \
            --user "$JAMYE_OBJECT_STORAGE_ACCESS_KEY_ID" >/dev/null || return 1
          last_stage=cleanup-policy-attach
          mc --quiet admin policy attach "$alias_name" jamye-server-cleanup \
            --user "$JAMYE_ACCOUNT_OBJECT_DELETION_ACCESS_KEY_ID" >/dev/null || return 1
          last_stage=complete
        }

        while [ "$SECONDS" -lt "$deadline" ]; do
          attempts=$((attempts + 1))
          if provision >/dev/null 2>&1; then
            echo "MinIO identities and policies ready after $attempts attempt(s)."
            exit 0
          fi
          sleep 1
        done

        echo "Timed out provisioning MinIO identities and policies after $attempts attempt(s); last stage: $last_stage." >&2
        exit 1
      '';
    };

    systemd.services.jamye-server-migrate = {
      description = "jamye-server forward-only SQLx migrations";
      after = ["postgresql.target"];
      requires = ["postgresql.target"];
      before = [
        "jamye-server-api.service"
        "jamye-server-worker.service"
      ];
      partOf = ["${serviceName}.target"];
      serviceConfig =
        commonHardening
        // {
          Type = "oneshot";
          RemainAfterExit = true;
          User = serviceName;
          Group = serviceName;
          WorkingDirectory = "/var/lib/${serviceName}";
          ExecStart = startMigration;
        };
    };

    systemd.services.jamye-server-api = {
      description = "jamye-server API";
      wants = ["network-online.target"];
      after = [
        "network-online.target"
        "postgresql.target"
        redisUnit
        "jamye-server-minio-identities.service"
        "jamye-server-migrate.service"
      ];
      requires = [
        "postgresql.target"
        redisUnit
        "jamye-server-minio-identities.service"
        "jamye-server-migrate.service"
      ];
      partOf = ["${serviceName}.target"];
      serviceConfig =
        commonHardening
        // {
          User = serviceName;
          Group = serviceName;
          WorkingDirectory = "/var/lib/${serviceName}";
          EnvironmentFile = baseEnvironmentFiles ++ mediaEnvironmentFiles;
          ExecStart = startApi;
          Restart = "on-failure";
          RestartSec = 2;
          ReadWritePaths = ["/var/lib/${serviceName}"];
        };
    };

    systemd.services.jamye-server-worker = {
      description = "jamye-server durable worker";
      wants = ["network-online.target"];
      after = [
        "network-online.target"
        "postgresql.target"
        redisUnit
        "jamye-server-minio-identities.service"
        "jamye-server-migrate.service"
      ];
      requires = [
        "postgresql.target"
        redisUnit
        "jamye-server-minio-identities.service"
        "jamye-server-migrate.service"
      ];
      partOf = ["${serviceName}.target"];
      serviceConfig =
        commonHardening
        // {
          User = serviceName;
          Group = serviceName;
          WorkingDirectory = "/var/lib/${serviceName}";
          EnvironmentFile = baseEnvironmentFiles ++ mediaEnvironmentFiles ++ cleanupEnvironmentFiles;
          ExecStart = startWorker;
          Restart = "on-failure";
          RestartSec = 2;
          ReadWritePaths = ["/var/lib/${serviceName}"];
        };
    };
  };
}
