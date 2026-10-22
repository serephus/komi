{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    naersk = {
      url = "github:nix-community/naersk";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nixit = {
      url = "github:serephus/nixit";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
      inputs.rust-overlay.follows = "rust-overlay";
      inputs.naersk.follows = "naersk";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      nixit,
      rust-overlay,
      naersk,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
    in
    {
      nixosModules.default = import ./nix/module.nix { inherit self; };

      githubRepositories.komi = nixit.lib.githubRepository {
        owner = "serephus";
        name = "komi";

        description = "A correspondence Go chess server.";
        homepage = "https://komi.sereph.us";
        topics = [
          "rust"
          "chess"
          "go-chess"
          "komi"
        ];
        visibility = "public";

        features = {
          wiki.enable = false;
          issues.enable = true;
          projects.enable = false;
          discussions.enable = false;
        };

        is_template = false;
        is_archived = false;

        pull = {
          merge.enable = true;
          squash.enable = false;
          rebase.enable = false;
          auto_merge = true;
          delete_branch_on_merge = true;
          update_branch = true;
        };

        actions = {
          enable = true;
          policy = "all";
          default_token_permissions = "read";
          allow_pr_approval = false;
        };

        rulesets = {
          default = {
            enforcement = "active";
            conditions.ref_name.include = [ "~DEFAULT_BRANCH" ];
            rules = [
              { type = "deletion"; }
              { type = "non_fast_forward"; }
            ];
          };
          pr = {
            enforcement = "active";
            conditions.ref_name.include = [ "~DEFAULT_BRANCH" ];
            rules = [
              {
                type = "required_status_checks";
                parameters = {
                  do_not_enforce_on_create = true;
                  strict_required_status_checks_policy = true;
                  required_status_checks = [
                    { "context" = "ubuntu-latest-x86_64-unknown-linux-gnu-nightly"; }
                    { "context" = "ubuntu-latest-x86_64-unknown-linux-gnu-stable"; }
                  ];
                };
              }
            ];
          };
        };
      };
    }
    // flake-utils.lib.eachSystem systems (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rustfmt"
            "clippy"
            "rust-analyzer"
          ];
        };
        naersk' = pkgs.callPackage naersk { };

        # Evaluating a system with the module enabled, so that `nix flake check`
        # fails if the module's options or its generated configuration break.
        komiModuleSystem = nixpkgs.lib.nixosSystem {
          inherit system;
          modules = [
            (import ./nix/module.nix { inherit self; })
            {
              services.komi.enable = true;
              system.stateVersion = "24.11";
            }
          ];
        };
      in
      {
        packages.default = naersk'.buildPackage ./.;

        checks.komi-nixos-module = komiModuleSystem.config.services.komi.configFile;

        devShell =
          with pkgs;
          mkShell {
            name = "komi";
            buildInputs = [ rust ];
          };
      }
    );
}
