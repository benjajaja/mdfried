use std::{
    cell::Cell,
    fs,
    path::PathBuf,
    sync::mpsc::Sender,
    time::{Duration, SystemTime},
};

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};

use crate::{Cmd, error::Error};

// Should take `tx: Sender<Event>` but that complains about some weird lifetime stuff.
pub fn watch(path: &PathBuf, tx: Sender<Cmd>) -> Result<Debouncer<RecommendedWatcher>, Error> {
    let parent = path
        .parent()
        .ok_or(Error::Generic(String::from("cannot watch without path")))?
        .to_owned();
    let filename = path
        .file_name()
        .ok_or(Error::Generic(String::from(
            "could not get filename part of path",
        )))?
        .to_owned();

    let mtime = Some(fs::metadata(path).and_then(|m| m.modified())?);
    let last_mtime: Cell<Option<SystemTime>> = Cell::new(mtime);
    let mtime_path = path.clone();

    // I can't believe we need to do this mtime check!
    // mtime resolution by platform heuristics:
    // - NTFS (Windows): 100ns - fine
    // - FAT32:          2s    - two quick saves could collapse
    // - APFS (macOS):   1ns   - fine
    // - HFS+ (old mac): 1s    - rare edge case
    // - ext4 (Linux):   1ns   - fine
    // - NFS/SMB:        varies, can lag on network mounts
    // TODO: make this configurable - or ignore it - who uses FAT32/HFS+ on this day an age?
    const DEBOUNCE_MILLISECONDS: u64 = 500;

    let mut debouncer = new_debouncer(
        Duration::from_millis(DEBOUNCE_MILLISECONDS),
        move |res: DebounceEventResult| match res {
            Ok(events) => {
                let dominated = events.iter().any(|e| e.path.file_name() == Some(&filename));
                if !dominated {
                    return;
                }

                let mtime = fs::metadata(&mtime_path).and_then(|m| m.modified()).ok();
                if mtime == last_mtime.get() {
                    return;
                }
                last_mtime.set(mtime);
                log::warn!("watch mtime changed, Cmd::FileChanged");
                if let Err(err) = tx.send(Cmd::FileChanged) {
                    log::error!("Failed to send Cmd::FileChanged: {err}");
                }
            }
            Err(err) => {
                log::error!("DebounceEventResult error: {err}");
            }
        },
    )?;
    log::info!("notify is watching {path:?}");
    debouncer
        .watcher()
        .watch(&parent, RecursiveMode::NonRecursive)?;
    Ok(debouncer)
}
