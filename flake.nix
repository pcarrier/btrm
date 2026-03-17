{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "wasm32-unknown-unknown" ];
        };

        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        browserWasm = rustPlatform.buildRustPackage {
          pname = "btrm-browser";
          version = "0.1.0";
          src = ./.;
          cargoBuildFlags = [ "-p" "btrm-browser" ];
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.wasm-pack pkgs.wasm-bindgen-cli pkgs.binaryen ];
          buildPhase = ''
            cd browser
            HOME=$TMPDIR wasm-pack build --target web --release --out-dir $out
          '';
          dontInstall = true;
          doCheck = false;
        };
      in
      {
        packages.default = rustPlatform.buildRustPackage {
          pname = "btrm-server";
          version = "0.1.0";
          src = ./.;
          cargoBuildFlags = [ "-p" "btrm-server" ];
          cargoLock.lockFile = ./Cargo.lock;
          preBuild = ''
            mkdir -p web
            cp ${browserWasm}/btrm_browser_bg.wasm web/
            cp ${browserWasm}/btrm_browser.js web/
          '';
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [
            rustToolchain
            pkgs.wasm-pack
            pkgs.wasm-bindgen-cli
            pkgs.binaryen
          ];

          shellHook = ''
            echo "btrm dev shell"
            echo "  build browser wasm: cd browser && wasm-pack build --target web --release --out-dir ../web"
            echo "  run server:         BTRM_PASS=secret cargo run --release -p btrm-server  # http://localhost:3264"
          '';
        };
      }
    );
}
