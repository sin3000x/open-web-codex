{
  description = "open-web-codex browser and platform server";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        packageJson = builtins.fromJSON (builtins.readFile ./package.json);

        frontend = pkgs.stdenv.mkDerivation {
          pname = "open-web-codex-frontend";
          version = packageJson.version;
          src = ./.;
          npmDeps = pkgs.importNpmLock { npmRoot = ./.; };
          nativeBuildInputs = [
            pkgs.nodejs_20
            pkgs.importNpmLock.npmConfigHook
          ];
          buildPhase = ''
            runHook preBuild
            npm run build
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            mkdir -p $out/share/open-web-codex
            cp -R dist/. $out/share/open-web-codex/
            runHook postInstall
          '';
        };

        server = pkgs.rustPlatform.buildRustPackage {
          pname = "open-web-codex-server";
          version = packageJson.version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "-p" "open-web-codex-server" ];
          doCheck = false;
          nativeBuildInputs = [ pkgs.makeWrapper ];
          postInstall = ''
            wrapProgram $out/bin/open-web-codex-server \
              --set-default OPEN_WEB_CODEX_WEB_DIST ${frontend}/share/open-web-codex
          '';
        };
      in {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            git
            nodejs_20
            openssl
            postgresql
            rust-analyzer
            rustc
            rustfmt
            rustPlatform.rustLibSrc
          ];
          shellHook = ''
            export RUST_SRC_PATH=${pkgs.rustPlatform.rustLibSrc}
          '';
        };

        formatter = pkgs.alejandra;
        packages.default = server;
        packages.frontend = frontend;
      });
}
