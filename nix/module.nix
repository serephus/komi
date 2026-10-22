# NixOS module for komi.
#
# The module writes komi's own TOML configuration file, so what the service runs
# with can always be inspected with:
#
#   systemctl cat komi
#   cat "$(systemctl show -p ExecStart --value komi | grep -o '/nix/store[^ ]*komi.toml')"
#
# The option names mirror the TOML keys so that the file, the options and the
# documentation all say the same thing.
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.komi;
  inherit (lib)
    mkIf
    mkOption
    mkEnableOption
    types
    optional
    optionalAttrs
    ;

  toml = pkgs.formats.toml { };

  # Option values, in the exact shape of komi's own configuration file.
  fromOptions = {
    bind = "${cfg.address}:${toString cfg.port}";
    database = "${cfg.stateDir}/komi.redb";
    site_name = cfg.siteName;
    log = cfg.log;
    trust_proxy = cfg.trustProxy;
    allow_registration = cfg.allowRegistration;
    default_language = cfg.defaultLanguage;
    refresh_seconds = cfg.refreshSeconds;

    session.days = cfg.sessionDays;

    lobby.stale_open_days = cfg.staleOpenDays;

    limits = {
      login_attempts = cfg.limits.loginAttempts;
      login_window_minutes = cfg.limits.loginWindowMinutes;
      registration_attempts = cfg.limits.registrationAttempts;
      registration_window_minutes = cfg.limits.registrationWindowMinutes;
    };
  }
  // optionalAttrs (cfg.secureCookies != null) {
    secure_cookies = cfg.secureCookies;
  };

  # Raw settings are merged last, so an operator can override any option and add
  # keys that have no option at all.
  settings = lib.recursiveUpdate fromOptions cfg.settings;

  configFile = toml.generate "komi.toml" settings;

  needsBindPort = cfg.port < 1024;
  defaultUser = "komi";
in
{
  options.services.komi = {
    enable = mkEnableOption "komi, a correspondence Go server with no clocks and no result";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "komi from this flake";
      description = "The komi package to run.";
    };

    address = mkOption {
      type = types.str;
      default = "127.0.0.1";
      example = "0.0.0.0";
      description = ''
        Address to listen on. The default keeps the server reachable only from
        the machine itself, which is what you want behind a reverse proxy. Set
        it to `0.0.0.0` to serve over the network directly, and then also
        consider {option}`services.komi.openFirewall`.
      '';
    };

    port = mkOption {
      type = types.port;
      default = 8080;
      description = "TCP port to listen on.";
    };

    siteName = mkOption {
      type = types.str;
      default = "komi";
      description = "Name shown in the page header and title.";
    };

    stateDir = mkOption {
      type = types.str;
      default = "/var/lib/komi";
      description = ''
        Directory holding the redb database. Created on boot, owned by
        {option}`services.komi.user`.
      '';
    };

    user = mkOption {
      type = types.str;
      default = defaultUser;
      description = "User to run the service as.";
    };

    group = mkOption {
      type = types.str;
      default = defaultUser;
      description = "Group to run the service as.";
    };

    secureCookies = mkOption {
      type = types.nullOr types.bool;
      default = null;
      description = ''
        Mark the session cookie `Secure`. The default leaves the decision to
        komi, which enables it unless it is listening on a loopback address.
        Set this to `true` when HTTPS ends at a reverse proxy that connects to
        komi over `127.0.0.1` — otherwise the browser would accept the session
        cookie over plain HTTP as well.
      '';
    };

    trustProxy = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Take the client address from `X-Forwarded-For` / `X-Real-IP` when rate
        limiting. Enable only when a reverse proxy sets those headers; otherwise
        a client could spoof them.
      '';
    };

    allowRegistration = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Whether new accounts may be created. Turn this off for a private
        instance after the accounts you need exist.
      '';
    };

    defaultLanguage = mkOption {
      type = types.enum [ "en" "zh" "ja" "ko" ];
      default = "en";
      description = ''
        Interface language for visitors who have not chosen one, and whose
        browser sends no preference. English, Chinese, Japanese or Korean.
      '';
    };

    refreshSeconds = mkOption {
      type = types.ints.between 5 3600;
      default = 30;
      description = ''
        Fallback refresh interval in seconds, used only by pages that cannot use
        the live update stream (a browser without `EventSource`).
      '';
    };

    sessionDays = mkOption {
      type = types.ints.between 1 3650;
      default = 30;
      description = "Days a session survives without being used.";
    };

    staleOpenDays = mkOption {
      type = types.ints.between 0 3650;
      default = 14;
      description = ''
        How long a game with a free seat keeps being listed in the lobby without
        activity. It stays reachable by its link either way. `0` never hides
        them.
      '';
    };

    log = mkOption {
      type = types.str;
      default = "info";
      description = ''
        Log filter in `RUST_LOG` syntax. `RUST_LOG` in the environment, if set,
        takes precedence.
      '';
    };

    limits = mkOption {
      default = { };
      description = "Rate limits that slow down password guessing and mass sign-ups.";
      type = types.submodule {
        options = {
          loginAttempts = mkOption {
            type = types.ints.positive;
            default = 10;
            description = "Failed logins from one address before it is locked out.";
          };
          loginWindowMinutes = mkOption {
            type = types.ints.between 1 1440;
            default = 15;
            description = "How long the login lockout lasts.";
          };
          registrationAttempts = mkOption {
            type = types.ints.positive;
            default = 5;
            description = "Accounts creatable from one address per window.";
          };
          registrationWindowMinutes = mkOption {
            type = types.ints.between 1 1440;
            default = 60;
            description = "The window for {option}`services.komi.limits.registrationAttempts`.";
          };
        };
      };
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = "Open {option}`services.komi.port` in the firewall.";
    };

    settings = mkOption {
      type = toml.type;
      default = { };
      example = {
        refresh_seconds = 30;
        limits = {
          login_attempts = 3;
        };
      };
      description = ''
        Raw configuration merged over the options above, using the key names of
        komi's own configuration file (see `config.example.toml` in the source).
        Use this for settings that have no option, or to override one.
      '';
    };

    configFile = mkOption {
      type = types.path;
      readOnly = true;
      default = configFile;
      defaultText = lib.literalMD "generated from the options above";
      description = ''
        The TOML file the service is started with. Read-only: change the options
        above instead. Useful for checking what the running service sees.
      '';
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.port != 0;
        message = "services.komi.port must not be 0.";
      }
      {
        assertion = cfg.user != "" && cfg.group != "";
        message = "services.komi.user and services.komi.group must not be empty.";
      }
    ];

    warnings =
      optional (cfg.openFirewall && (cfg.address == "127.0.0.1" || cfg.address == "::1"))
        "services.komi.openFirewall is enabled but services.komi.address is a loopback address, so the port will not be reachable from outside anyway."
      ++ optional (!cfg.openFirewall && cfg.address != "127.0.0.1" && cfg.address != "::1")
        "services.komi listens on ${cfg.address} but services.komi.openFirewall is off; make sure the port is opened elsewhere if it should be reachable.";

    users.users = mkIf (cfg.user == defaultUser) {
      ${defaultUser} = {
        isSystemUser = true;
        group = cfg.group;
        home = cfg.stateDir;
        description = "komi service user";
      };
    };

    users.groups = mkIf (cfg.group == defaultUser) {
      ${defaultUser} = { };
    };

    systemd.tmpfiles.rules = [
      "d '${cfg.stateDir}' 0750 ${cfg.user} ${cfg.group} -"
    ];

    systemd.services.komi = {
      description = "komi correspondence Go server";
      documentation = [ "https://github.com/serephus/komi" ];
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      wants = [ "network.target" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = "${cfg.package}/bin/komi --config ${configFile}";
        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.stateDir;
        Restart = "on-failure";
        RestartSec = 5;
        # The database and its WAL files are the only things written.
        ReadWritePaths = [ cfg.stateDir ];
        UMask = "0077";

        # Boring, well understood hardening. Deliberately no SystemCallFilter:
        # File locking and fsync paths are exactly the kind of thing a hand
        # written allow-list gets subtly wrong, and a crash loop is a worse
        # outcome than the extra isolation would have been.
        NoNewPrivileges = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        ProtectClock = true;
        ProtectHostname = true;
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        RemoveIPC = true;
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
        SystemCallArchitectures = "native";
      }
      // optionalAttrs needsBindPort {
        AmbientCapabilities = [ "CAP_NET_BIND_SERVICE" ];
        CapabilityBoundingSet = [ "CAP_NET_BIND_SERVICE" ];
      }
      // optionalAttrs (!needsBindPort) {
        CapabilityBoundingSet = [ ];
      };
    };

    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];
  };
}
