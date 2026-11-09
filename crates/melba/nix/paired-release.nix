{ stdenv, fetchurl, autoPatchelfHook, makeWrapper, lib, alsa-lib, dbus, webkitgtk_4_1, gtk3, libxkbcommon, wayland, libGL, libpulseaudio, libX11, libXcursor, libXi, libXrandr }:
let
  version = "0.3.26";
  aarch64 = stdenv.hostPlatform.isAarch64;
  target = if aarch64 then "aarch64-unknown-linux-gnu" else "x86_64-unknown-linux-gnu";
  folder = "melba-v${version}-${target}";
  hash = if aarch64 then
    "sha256-9dEIvvd2iOWfpwiHKM34KHM+KW1yfemEwGKNo8F/R7c="
  else
    "sha256-r1H+jPMLwy+4EcXFzp2zVhugp5gGU9ORee59nA5+feQ=";
  runtimeLibraries = [
    alsa-lib dbus webkitgtk_4_1 gtk3 libxkbcommon wayland libGL libpulseaudio
    libX11 libXcursor libXi libXrandr stdenv.cc.cc.lib
  ];
in
stdenv.mkDerivation {
  pname = "melba-paired";
  inherit version;
  src = fetchurl {
    url = "https://github.com/nekomario28/melba/releases/download/v${version}/${folder}.tar.gz";
    inherit hash;
  };
  nativeBuildInputs = [ autoPatchelfHook makeWrapper ];
  buildInputs = runtimeLibraries;
  unpackPhase = ''
    tar -xzf "$src"
    cd "${folder}"
  '';
  dontBuild = true;
  installPhase = ''
    runHook preInstall
    install -Dm755 melba "$out/bin/melba"
    install -Dm755 melbad "$out/bin/melbad"
    for file in NOTICE.md LICENSE-MIT LICENSE-APACHE SPOTIFAST-LICENSE melba-THIRD-PARTY.txt melbad-THIRD-PARTY.txt receipt.json; do
      if test -f "$file"; then
        install -Dm644 "$file" "$out/share/licenses/melba/$file"
      fi
    done
    install -Dm644 ${../packaging/melba.desktop} "$out/share/applications/melba.desktop"
    install -Dm644 ${../packaging/icons/melba.svg} "$out/share/icons/hicolor/scalable/apps/melba.svg"
    runHook postInstall
  '';
  postFixup = ''
    wrapProgram "$out/bin/melba" --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibraries}"
    wrapProgram "$out/bin/melbad" --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibraries}"
  '';
  meta = {
    description = "Melba GUI and matching playback daemon from a checksum-pinned release";
    homepage = "https://github.com/nekomario28/melba";
    license = lib.licenses.mit;
    mainProgram = "melba";
    platforms = lib.platforms.linux;
  };
}
