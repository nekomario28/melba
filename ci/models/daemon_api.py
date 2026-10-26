# crates/melba ApiRequest -> ApiResponse — the GUI's contract with the daemon
# worker (src/daemon_worker.rs `handle_api` / `catalog_answer`). `q.X` = the app
# sends variant X; `a.Y` = the worker emits response variant Y.
#
# Every request the app can send is answered: the podcast/show/episode
# surface was removed (Apple Music has no podcasts), and the vestigial
# remote-playback commands were cut in favour of PlayerCommand. UNANSWERED
# and DEAD_RESPONSES stay as pinned empty sets so ci/check.sh's drift guard
# fails the moment a new send-only variant appears.
import protogx as p

ANSWERED = {
    "Me": "Me",
    "Devices": "Devices",
    "AudioDevices": "AudioDevices",
    "SetAudioDevice": "AudioDeviceSet",
    "PlaybackState": "PlaybackState",
    "Queue": "Queue",
    "AddToQueue": "QueueAdded",
    "AddManyToQueue": "QueueBatchAdded",
    "RecentlyPlayed": "RecentlyPlayed",
    "Search": "Search",
    "PlaySearch": "PlaySearch",
    "Track": "Track",
    "Album": "Album",
    "AlbumTracks": "AlbumTracks",
    "AlbumQueueTracks": "AlbumQueueTracks",
    "Artist": "Artist",
    "ArtistTopTracks": "ArtistTopTracks",
    "ArtistAlbums": "ArtistAlbums",
    "RelatedArtists": "RelatedArtists",
    "Playlist": "Playlist",
    "PlaylistItems": "PlaylistItems",
    "MyPlaylists": "MyPlaylists",
    "MadeForYou": "MadeForYou",
    "Contains": "Contains",
    "SetSaved": "SavedChanged",
    "SavedTracks": "SavedTracks",
    "SavedAlbums": "SavedAlbums",
    "FollowedArtists": "FollowedArtists",
    "TopTracks": "TopTracks",
    "TopArtists": "TopArtists",
    "Recommendations": "Recommendations",
    "CreatePlaylist": "PlaylistCreated",
    "AddToPlaylist": "PlaylistItemsChanged",
    "RemoveFromPlaylist": "PlaylistItemsChanged",
    "ReorderPlaylist": "PlaylistItemsChanged",
    "CheckPlaylistDuplicates": "PlaylistDuplicatesChecked",
    "FollowPlaylist": "PlaylistFollowChanged",
    "UpdatePlaylist": "PlaylistUpdated",
    "UploadPlaylistCover": "PlaylistCoverUploaded",
}

# Sent but never answered (`_ => None` in catalog_answer): pinned empty —
# any new entry here is a send the UI can wait on forever.
UNANSWERED = []

# ApiResponse enum variants no producer emits anywhere in the crate — pinned
# empty so the drift guard can tell "known dead surface" from a new hole.
DEAD_RESPONSES = []

requests = sorted(ANSWERED)
replies = sorted(set(ANSWERED.values()))

protocol = p.Protocol(
    states=["Idle", "Pending", "Shutdown"],
    start="Idle",
    terminal=["Shutdown"],
    messages=(
        [f"q.{q}" for q in requests + UNANSWERED]
        + [f"a.{a}" for a in replies]
        + ["Quit"]
    ),
    transitions=(
        [("Idle", "Pending", f"q.{q}") for q in requests]
        # answered requests come back to Idle on their reply; unanswered
        # ones are self-loops — nothing pending can ever depend on them.
        + [("Idle", "Idle", f"q.{q}") for q in UNANSWERED]
        + [("Pending", "Idle", f"a.{a}") for a in replies]
        + [("Idle", "Shutdown", "Quit")]
    ),
    requests=[f"q.{q}" for q in requests],
    responses={f"q.{q}": f"a.{a}" for q, a in ANSWERED.items()},
)
