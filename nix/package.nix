{ lib, rustPlatform, pkg-config, zstd, git }:

rustPlatform.buildRustPackage {
  pname = "iliada-tools";
  version = lib.trim (builtins.readFile ../VERSION);

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../.cargo
      ../crates
      ../xtask
      # iliada-system's tests check the configuration it writes from these.
      ../templates
      # Actions Iliada's own apps declare for Homer (intents.rs).
      ../nix/intents
    ];
  };
  cargoLock = {
    lockFile = ../Cargo.lock;
    # cosmic-protocols (Search switches windows with COSMIC's toplevel
    # management), the same revision the desktop apps use.
    allowBuiltinFetchGit = true;
  };

  # zstd: used by agora's tests (archive fixtures) and the catalog builder.
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ zstd ];
  # iliada-system's tests drive git (the configuration is a repository).
  nativeCheckInputs = [ git ];

  # Only the default members (crates/*) are built; xtask is dev tooling.

  postInstall = ''
    install -Dm644 -t $out/share/iliada/intents nix/intents/*.toml
  '';

  meta = {
    description = "Iliada system tools: the system service, agora, broker, sandbox and the iliada command";
    homepage = "https://iliada-os.com";
    license = with lib.licenses; [ mit asl20 ];
    platforms = lib.platforms.unix;
    mainProgram = "iliada-broker";
  };
}
