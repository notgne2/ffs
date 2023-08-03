{
  nixConfig = {
    flake-registry = "https://github.com/serokell/flake-registry/raw/master/flake-registry.json";
  };

  inputs = {
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };

    flake-utils.url = "github:numtide/flake-utils";

    naersk.url = "github:nix-community/naersk";

    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, crate2nix, flake-utils, nix, flake-compat, rust-overlay, naersk }:
    flake-utils.lib.eachSystem [ "x86_64-linux" ]
      (system:
        let
          pkgs = import nixpkgs { inherit system; overlays = [ rust-overlay.overlay ]; };

          naersk-lib = naersk.lib."${system}".override {
            cargo = pkgs.rust-bin.nightly.latest.default;
            rustc = pkgs.rust-bin.nightly.latest.default;
          };

          nix' = nix.defaultPackage.${system};
        in
        {
          packages.ffs = naersk-lib.buildPackage {
            pname = "ffs";

            src = ./.;

            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs = with pkgs; [
              fuse
              file
              sqlite
            ];
          };

          packages.default = self.packages.${system}.ffs;
          defaultPackage = self.packages.${system}.default;

          checks = {
            trailing-whitespace = pkgs.build.checkTrailingWhitespace ./.;
          };

          devShell = pkgs.mkShell {
            RUST_LOG = "info,ffs=debug";
            inputsFrom = builtins.attrValues self.packages.${system};
            FFS_MAGIC_FILE = "${pkgs.file}/share/misc/magic.mgc";
            buildInputs = with pkgs; [
              nix'
              (pkgs.rust-bin.nightly.latest.default.override {
                extensions = [ "rust-src" "rust-analyzer-preview" "rustfmt" "rls" ];
              })
              diesel-cli
            ];
          };
        }) // { };
}
