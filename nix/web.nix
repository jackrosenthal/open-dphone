# The web frontend: TypeScript, React, Tailwind, Vite.  Dependencies come
# from web/package-lock.json; the Connect client code is generated from
# proto/ by buf with the plugins in node_modules, so nothing is fetched
# during the build.  Output: the Vite `dist/` directory.
{
  lib,
  buildNpmPackage,
  buf,
  npmDepsHash,
}:
let
  root = ../.;
in
buildNpmPackage {
  pname = "dphone-web-frontend";
  version = "3.0.0";
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/web")
      (root + "/proto")
    ];
  };
  sourceRoot = "source/web";
  inherit npmDepsHash;
  nativeBuildInputs = [ buf ];
  # `npm run build` is expected to run code generation, type checking, and
  # `vite build`.
  installPhase = ''
    runHook preInstall
    cp -r dist $out
    runHook postInstall
  '';
}
