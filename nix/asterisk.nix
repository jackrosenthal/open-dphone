# Asterisk with `res_open_dpma` (Open DPMA, open-dpma/ in this repository), for
# the integration test that provisions and registers the firmware's main role
# against a real server (crates/opendeskphone/tests/asterisk.rs).
#
# `res_open_dpma` generates its bindings from the configured headers of the
# Asterisk it is loaded into, including the bundled pjproject's, which an
# Asterisk installation does not carry.  So Asterisk is built here from
# the nixpkgs recipe with one addition: the configured headers are kept,
# in the layout the module's build reads (`ASTERISK_SOURCE_DIR`).
{
  asterisk,
  runCommand,
  module, # res_open_dpma built against `server`'s headers (workspace.nix)
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
in
{
  inherit server;

  # Asterisk's module directory with `res_open_dpma` added, as `astmoddir`
  # of the test instance.
  modules = runCommand "asterisk-modules-with-res_open_dpma" { } ''
    mkdir $out
    ln -s ${server}/lib/asterisk/modules/* $out/
    ln -s ${module}/lib/libres_open_dpma.so $out/res_open_dpma.so
  '';
}
