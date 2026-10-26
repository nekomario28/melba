# melbad module graph — mirrors `mod`/`use crate::` in src/ and `use melbad::`
# in the bins. Declared graph; drift guard in ci/check.sh greps pub mod
# names from lib.rs and demands each appear here.
import depsgx as d

graph = d.ModuleGraph(
    modules=[
        "main", "melba_shot",
        "player", "stream", "musickit", "wrapper", "auth_gui", "auth_webview",
        "library",
        "audio", "auth", "config", "frontend",
    ],
    depends=[
        ("main", "audio"),
        ("main", "auth"),
        ("main", "auth_gui"),
        ("main", "auth_webview"),
        ("main", "config"),
        ("main", "library"),
        ("main", "musickit"),
        ("main", "player"),
        ("main", "stream"),
        ("main", "wrapper"),
        ("player", "audio"),
        ("player", "config"),     # xdg_dir for art cache + queue state
        ("player", "frontend"),   # bounded desktop CLI/IPC handoff
        ("player", "library"),    # file_meta for embedded tags/cover
        ("player", "musickit"),   # preview: resolution + catalog enrich
        ("player", "stream"),     # stream:/preview: open + queue uris
        ("stream", "audio"),      # probe/open shared with local playback
        ("stream", "auth"),       # token presence picks stream: vs preview:
        ("stream", "config"),     # wrapper_mode / wrapper_url / cdm_path
        ("stream", "musickit"),   # storefront + aac/aac-hls endpoints
        ("stream", "wrapper"),    # managed-mode spawn + alac decryptor
        ("wrapper", "config"),    # wrapper_mode + check uses config only
        ("library", "audio"),     # read_tags -> probe_source
        ("musickit", "auth"),     # music-user-token storage
        ("musickit", "config"),   # xdg_dir for dev-token/preview cache
        ("musickit", "library"),  # sone.collection writes during enrich
        ("auth", "config"),       # xdg_dir for token dir + profile roots
        # local sign-in: owns the session lock, the SDK bridge, and the
        # session-derived token's validation
        ("auth_gui", "auth"),
        ("auth_gui", "config"),
        ("auth_gui", "musickit"),
        ("auth_gui", "wrapper"),
        ("auth_webview", "auth"),
        ("auth_webview", "config"),    # xdg_dir for webview profile dir
        ("auth_webview", "musickit"),  # validate_token probe
    ],
    roots=["main", "melba_shot"],
    layers=[
        ["main", "melba_shot"],                            # binaries / entry points
        ["player", "stream", "musickit", "wrapper",
         "auth_gui", "auth_webview"],                      # services
        ["library"],                                       # contract layer
        ["auth"],                                          # token store: needs config paths only
        ["audio", "config", "frontend"],                   # leaves: depend on nothing
    ],
)
