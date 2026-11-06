//! A playlist prefix on disk, keyed by the snapshot it was written under.
//!
//! What is stored is only ever a prefix: a page range starting at zero, with
//! the offset the next page would continue from. The runtime type
//! (`model::PlaylistCache`) is not serialized directly, so a file names its own
//! version and the account it belongs to; a file that disagrees with either,
//! or whose snapshot no longer matches the service's, is simply ignored.

use serde::{Deserialize, Serialize};

use crate::api::models::PlaylistItem;
use crate::model::PlaylistCache;

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct File {
    v: u32,
    account: String,
    snapshot: String,
    total: u32,
    next_offset: Option<u32>,
    items: Vec<PlaylistItem>,
}

/// The prefix on disk for this account, with the snapshot it was written
/// under. Whether that snapshot still describes the list is the caller's
/// question: the app already compares it against the service's answer before
/// adopting anything, and asking twice only creates a second owner that can
/// answer "no" on the first open of a session.
pub fn read(path: &std::path::Path, account: &str) -> Option<PlaylistCache> {
    let bytes = std::fs::read(path).ok()?;
    let file: File = serde_json::from_slice(&bytes).ok()?;
    if file.v != VERSION || file.account != account {
        return None;
    }
    // A prefix that claims more rows than it holds, or a next offset that does
    // not continue from its own end, would silently drop songs.
    if file.items.is_empty()
        || file.items.len() > file.total as usize
        || file
            .next_offset
            .is_some_and(|next| next as usize != file.items.len())
    {
        return None;
    }
    Some(PlaylistCache {
        snapshot: file.snapshot,
        items: file.items,
        total: file.total,
        next_offset: file.next_offset,
        appendable: true,
    })
}

/// Write the prefix atomically; a half-written file must never be read back.
pub fn write(
    path: &std::path::Path,
    account: &str,
    snapshot: &str,
    total: u32,
    next_offset: Option<u32>,
    items: Vec<PlaylistItem>,
) -> std::io::Result<()> {
    let file = File {
        v: VERSION,
        account: account.to_string(),
        snapshot: snapshot.to_string(),
        total,
        next_offset,
        items,
    };
    let bytes = serde_json::to_vec(&file).map_err(std::io::Error::other)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    fastframe_file::write(path, bytes)
}

/// The rows a store should leave on disk: the existing prefix when the caller
/// says this is an append, otherwise exactly what it was given.
pub fn rows_to_write(
    existing: Option<Vec<PlaylistItem>>,
    rows: crate::backend::PlaylistCacheRows,
) -> Vec<PlaylistItem> {
    use crate::backend::PlaylistCacheRows;
    match rows {
        PlaylistCacheRows::Replace(items) => items,
        PlaylistCacheRows::Append {
            previous_rows,
            previous_offset,
            items,
        } => {
            // Only append onto a prefix that still ends where the caller says
            // it does; anything else is written as a fresh prefix.
            match existing {
                Some(mut held)
                    if held.len() == previous_rows && previous_offset as usize == previous_rows =>
                {
                    held.extend(items);
                    held
                }
                _ => items,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::{PlayableItem, Track};

    fn item(n: u32) -> PlaylistItem {
        PlaylistItem {
            item: Some(PlayableItem::Track(Track {
                uri: format!("melba:track:{n}"),
                ..Track::default()
            })),
            ..PlaylistItem::default()
        }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mlb-plc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("playlist.json")
    }

    #[test]
    fn a_prefix_round_trips_for_the_account_and_snapshot_that_wrote_it() {
        let path = tmp("roundtrip");
        write(
            &path,
            "alice",
            "snap-1",
            10,
            Some(2),
            vec![item(1), item(2)],
        )
        .unwrap();
        let cache = read(&path, "alice").expect("own prefix");
        assert_eq!(cache.items.len(), 2);
        assert_eq!(cache.next_offset, Some(2));
        assert_eq!(cache.snapshot, "snap-1", "the file keeps its snapshot");
        assert!(cache.appendable);
        assert!(read(&path, "bob").is_none(), "another account");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_prefix_that_does_not_continue_from_its_own_end_is_a_miss() {
        let path = tmp("incoherent");
        write(&path, "alice", "s", 10, Some(5), vec![item(1), item(2)]).unwrap();
        assert!(read(&path, "alice").is_none());
        write(&path, "alice", "s", 1, None, vec![item(1), item(2)]).unwrap();
        assert!(read(&path, "alice").is_none(), "more rows than total");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_append_lands_only_on_the_prefix_it_was_asked_to_extend() {
        use crate::backend::PlaylistCacheRows;
        let held = vec![item(1), item(2)];
        let appended = rows_to_write(
            Some(held.clone()),
            PlaylistCacheRows::Append {
                previous_rows: 2,
                previous_offset: 2,
                items: vec![item(3)],
            },
        );
        assert_eq!(appended.len(), 3);

        let mismatched = rows_to_write(
            Some(held),
            PlaylistCacheRows::Append {
                previous_rows: 7,
                previous_offset: 7,
                items: vec![item(9)],
            },
        );
        assert_eq!(
            mismatched.len(),
            1,
            "a foreign prefix is replaced, not merged"
        );

        let replaced = rows_to_write(
            Some(vec![item(1)]),
            PlaylistCacheRows::Replace(vec![item(4), item(5)]),
        );
        assert_eq!(replaced.len(), 2);
    }

    #[test]
    fn a_corrupt_file_is_a_miss() {
        let path = tmp("corrupt");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(read(&path, "alice").is_none());
        std::fs::write(&path, b"[]").unwrap();
        assert!(read(&path, "alice").is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
