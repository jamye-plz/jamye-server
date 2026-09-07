{
  jamyeServerModule,
  pkgs,
}:
pkgs.testers.runNixOSTest {
  name = "jamye-server-stack";

  nodes.machine = {
    lib,
    pkgs,
    ...
  }: {
    imports = [jamyeServerModule];

    system.stateVersion = "25.11";
    networking.useDHCP = false;
    virtualisation = {
      cores = 4;
      memorySize = 3072;
    };

    # Keep the smoke test usable on Linux builders without KVM. PostgreSQL's
    # first cluster initialization can exceed the NixOS default under TCG.
    systemd.services.postgresql.serviceConfig.TimeoutStartSec =
      lib.mkForce "15min";

    environment.systemPackages = [
      pkgs.curl
      pkgs.jq
    ];

    environment.etc = {
      "jamye-server/runtime.env" = {
        mode = "0400";
        text = ''
          JAMYE_ACCESS_TOKEN_SECRET=test-only-00000000000000000000000000000000
          JAMYE_ACCESS_TOKEN_ISSUER=jamye-nixos-smoke
          JAMYE_ACCESS_TOKEN_AUDIENCE=jamye-nixos-smoke-client
        '';
      };
      "jamye-server/media.env" = {
        mode = "0400";
        text = ''
          JAMYE_OBJECT_STORAGE_ACCESS_KEY_ID=jamye-smoke-media
          JAMYE_OBJECT_STORAGE_SECRET_ACCESS_KEY=jamye-smoke-media-secret
        '';
      };
      "jamye-server/cleanup.env" = {
        mode = "0400";
        text = ''
          JAMYE_ACCOUNT_OBJECT_DELETION_ACCESS_KEY_ID=jamye-smoke-cleanup
          JAMYE_ACCOUNT_OBJECT_DELETION_SECRET_ACCESS_KEY=jamye-smoke-cleanup-secret
        '';
      };
      "jamye-server/minio-root.env" = {
        mode = "0400";
        text = ''
          MINIO_ROOT_USER=jamye-smoke-root
          MINIO_ROOT_PASSWORD=jamye-smoke-root-secret
        '';
      };
    };

    services.jamye-server = {
      enable = true;
      listenAddress = "127.0.0.1:8080";
      environmentFile = "/etc/jamye-server/runtime.env";
      objectStorage = {
        publicEndpoint = "https://media.example.test";
        mediaCredentialsFile = "/etc/jamye-server/media.env";
        cleanupCredentialsFile = "/etc/jamye-server/cleanup.env";
        rootCredentialsFile = "/etc/jamye-server/minio-root.env";
      };
    };
  };

  testScript = ''
    machine.start()
    machine.wait_for_unit("jamye-server.target")

    for unit in (
        "postgresql.service",
        "redis-jamye-server.service",
        "minio.service",
        "jamye-server-minio-identities.service",
        "jamye-server-migrate.service",
        "jamye-server-api.service",
        "jamye-server-worker.service",
    ):
        machine.succeed(f"systemctl is-active {unit}")

    machine.wait_for_open_port(8080)
    machine.wait_until_succeeds(
        "curl -fsS http://127.0.0.1:8080/health/live | jq -e '.status == \"live\"'"
    )
    machine.wait_until_succeeds(
        "curl -fsS http://127.0.0.1:8080/health/ready "
        "| jq -e '.status == \"ready\" "
        "and .checks.postgres.status == \"ready\" "
        "and .checks.redis.status == \"ready\" "
        "and .checks.minio.status == \"ready\"'"
    )
  '';
}
