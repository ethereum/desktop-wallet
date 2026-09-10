self: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.edw;
in {
  options.programs.edw = {
    enable = lib.mkEnableOption "the edw Ethereum desktop wallet";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "edw.packages.\${system}.default";
      description = "The edw package to install.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [cfg.package];
  };
}
