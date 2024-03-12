{
  description = "rust workspace";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    let
      rust-version = "1.71.0";
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ rust-overlay.overlays.default ];
        pkgs = import nixpkgs { inherit system overlays; };
        lib = pkgs.lib;

        buildInputs = with pkgs; [
          (rust-bin.stable.${rust-version}.default.override {
              extensions =
                [ "rust-src" "llvm-tools-preview" "rust-analysis" ];
          })
          jq
          nixos-shell
          wget
          mold-wrapped
          protobuf
        ];
        nativeBuildInputs = with pkgs; [ pkg-config nixpkgs-fmt ];
        libs = with pkgs; [];
      in
      rec {
        devShell = with pkgs;
          mkShell {
            buildInputs = [ ] ++ buildInputs;
            inherit nativeBuildInputs;

            shellHook = ''
            
            '';
          };
      });
}
