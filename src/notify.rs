use std::{
    cell::Cell,
    fs,
    path::PathBuf,
    sync::mpsc::Sender,
    time::{Duration, SystemTime},
};

use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer, notify::*};

use crate::Cmd;

pub fn watch(path: &PathBuf, tx: Sender<Cmd>) -> Result<Debouncer<RecommendedWatcher>> {
    let parent = path.parent().unwrap().to_owned();
    let filename = path.file_name().unwrap().to_owned();
    let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
    let last_mtime: Cell<Option<SystemTime>> = Cell::new(mtime);
    let path_clone = path.clone();
    let mut debouncer =
        new_debouncer(
            Duration::from_secs(1),
            move |res: DebounceEventResult| match res {
                Ok(events) => {
                    let dominated = events.iter().any(|e| e.path.file_name() == Some(&filename));
                    if !dominated {
                        return;
                    }

                    let mtime = fs::metadata(&path_clone).and_then(|m| m.modified()).ok();
                    if mtime == last_mtime.get() {
                        log::debug!("mtime unchanged, skipping");
                        return;
                    }
                    log::warn!("mtime changed: {:?}", mtime);
                    last_mtime.set(mtime);
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
