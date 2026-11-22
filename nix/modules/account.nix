# The Iliada account on each computer (docs/account.md): which account server
# people sign in to (Settings → Account, `iliada account`), and an hourly
# backup of each signed-in person's settings and the computer's apps and
# tools (also moments after those change), encrypted on the computer before
# it is sent. People who haven't signed in are left alone.
#
# Sign-in is off until the public account service is hosted for real. Turn
# it back on by setting the default below to the official address (one line).
{ config, lib, ... }:
let
  cfg = config.iliada.account;
  pkg = config.iliada.package;
  # The public Iliada account. Used when `server` is set; kept here so
  # turning sign-in back on is a one-line change of the option default.
  officialServer = "https://account.iliada-os.com";
in
{
  options.iliada.account.server = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    # Off until the public account service has a host that stays up.
    # Re-enable: default = officialServer;
    default = null;
    example = officialServer;
    description = ''
      The Iliada account people sign in to (github.com/iliada-os/cloud):
      Iliada's own, or one you run yourself. null (the default) hides
      sign-in and shows that accounts are coming soon; people already
      signed in keep their account and hourly backup (the address is
      saved on the computer). `iliada account sign-in --server …` still
      works.
    '';
  };

  config = {
    environment.sessionVariables = lib.mkIf (cfg.server != null) {
      ILIADA_ACCOUNT_SERVER = cfg.server;
    };
    # Always: people already signed in keep hourly backup after an update
    # that turns sign-in off. The command does nothing if they aren't.
    systemd.user.services.iliada-account-backup = {
      description = "Back up your settings to your Iliada account";
      environment = lib.optionalAttrs (cfg.server != null) {
        ILIADA_ACCOUNT_SERVER = cfg.server;
      };
      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${pkg}/bin/iliada account backup --if-signed-in";
        Nice = 10;
      };
    };
    # The computer's apps and tools go to the account moments after they
    # change, not only hourly (an unchanged backup uploads nothing).
    systemd.user.paths.iliada-account-backup = {
      wantedBy = [ "default.target" ];
      pathConfig.PathChanged = [
        "/etc/iliada/settings.toml"
        "/etc/iliada/apps.toml"
      ];
    };
    systemd.user.timers.iliada-account-backup = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnStartupSec = "10min";
        OnUnitActiveSec = "1h";
        RandomizedDelaySec = "5min";
      };
    };
  };
}
