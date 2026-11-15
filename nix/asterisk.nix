# Asterisk with `res_dpma_rs` (the open DPMA module, github.com/jackrosenthal/dpma-rs),
# for the integration test that provisions and registers the firmware's
# main role against a real server (crates/dphone/tests/asterisk.rs).
#
# `res_dpma_rs` generates its bindings from the configured headers of the
# Asterisk it is loaded into, including the bundled pjproject's, which an
# Asterisk installation does not carry.  So Asterisk is built here from
# the nixpkgs recipe with one addition: the configured headers are kept,
# in the layout the module's build reads (`ASTERISK_SOURCE_DIR`).
{
  lib,
  asterisk,
  fetchFromGitHub,
  rustPlatform,
  runCommand,
}:
let
  server = asterisk.overrideAttrs (old: {
    postInstall = (old.postInstall or "") + ''
      mkdir -p $out/share/asterisk-source
      cp -a --parents include \
        third-party/pjproject/source/pjlib/include \
        third-party/pjproject/source/pjlib-util/include \
        third-party/pjproject/source/pjnath/include \
        third-party/pjproject/source/pjmedia/include \
        third-party/pjproject/source/pjsip/include \
        $out/share/asterisk-source
      test -f $out/share/asterisk-source/include/asterisk/buildopts.h
      test -f $out/share/asterisk-source/third-party/pjproject/source/pjlib/include/pj/config_site.h
    '';
  });

  module = rustPlatform.buildRustPackage (finalAttrs: {
    pname = "res_dpma_rs";
    version = "0.3.4";
    src = fetchFromGitHub {
      owner = "jackrosenthal";
      repo = "dpma-rs";
      tag = "v${finalAttrs.version}";
      hash = "sha256-/Wtvp16xPJioRrKYNyOvzugeHwb3I2AOPSjkJyLIJAw=";
    };
    cargoHash = "sha256-FQewHoHhiWQD1vNucvwUT9DYwM6fKcALTS5NvJr/bB4=";
    nativeBuildInputs = [ rustPlatform.bindgenHook ];
    ASTERISK_SOURCE_DIR = "${server}/share/asterisk-source";
    cargoBuildFlags = [
      "-p"
      "res_dpma_rs"
    ];
    # The module's own tests cannot link outside Asterisk.
    doCheck = false;
    installPhase = ''
      runHook preInstall
      install -D -m 0755 "$(find target -name libres_dpma_rs.so -path '*release*' | head -n 1)" \
        $out/lib/asterisk/modules/res_dpma_rs.so
      runHook postInstall
    '';
    meta.license = lib.licenses.gpl2Plus;
  });
in
{
  inherit server module;

  # Asterisk's module directory with `res_dpma_rs` added, as `astmoddir`
  # of the test instance.
  modules = runCommand "asterisk-modules-with-res_dpma_rs" { } ''
    mkdir $out
    ln -s ${server}/lib/asterisk/modules/* $out/
    ln -s ${module}/lib/asterisk/modules/res_dpma_rs.so $out/
  '';
}
