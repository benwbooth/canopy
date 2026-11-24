# canopy

An open-source Google Drive sync client for Linux, written in Rust with a
Qt Quick window, made to replace Insync on the arbor/hearth/grove desktop.

- Real-time two-way sync (Drive changes feed + inotify), one-way modes
  (cloud → computer, computer → cloud backup), selective sync, sync any
  local folder.
- Google Docs as link files or converted to Office/OpenDocument, with local
  edits going back into the Google file.
- Conflicts kept safe (keep both by default), deletions always go to a
  trash, a missing or emptied folder never deletes anything in the cloud.
- Tray icon, sync badges and a Drive menu in grove, `canopy` command line,
  `canopyd` runs without a display.

See [DESIGN.md](DESIGN.md) for Insync's feature list and how canopy is
built. Work in progress.

## Google sign-in

Google only lets verified apps use the full Drive permission, so canopy
signs in through your own Google Cloud OAuth client (as rclone does). The
window's Help explains the five-minute setup.
