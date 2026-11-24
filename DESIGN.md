# canopy design

canopy is an open-source Google Drive sync client for Ben's desktop (arbor,
hearth, grove), written to replace Insync. Insync is the template: its
feature set is listed below, and its sync engine's structure was read from
an installed Insync 3.9.8 (its SQLite schemas, worker names and logs; no
code was decompiled).

## How Insync works (what canopy copies)

Insync is a frozen Python 3.9 + PySide2 app with a QtWebEngine UI.

- `settings.db`: `accounts` (id, cloud, email, OAuth extras) and app
  `settings` (paused, window geometry, activity filters...).
- `data/gd-<account>.db`, the sync state:
  - `nodes(node_id, parent_id, sync_flags)`: the tree of items Insync
    knows about, with a local half (`fs_items`: name, type, mtime, size,
    inode) and a cloud half (`cl_items`: id, name, type, mtime, size,
    mime, checksum) per node. Comparing the two halves with the last
    synced state is a three-way merge.
  - `sync_choices(node_id, chosen_sync_flags, chosen_location)`: selective
    sync choices and custom ("@") locations outside the base folder.
  - `sync_prefs`: per account, `EXPORT_OPTION` (LINK / MS_OFFICE /
    OPEN_DOCUMENT) and `CONFLICT_RESOLUTION_OPTION` with four cases:
    LOCAL_EDIT_CLOUD_EDIT, LOCAL_EDIT_CLOUD_DELETE, LOCAL_EDIT_UNSYNC and
    LOCAL_DELETE_CLOUD_EDIT, each ALWAYS_ASK or a fixed choice.
  - `pending_restores`, `paused_backup_paths`, `last_backup_time`:
    local-to-cloud backups. `starred_items`.
- `live/live-gd-<account>.db`, the cloud mirror:
  - `cl_resources(cl_id, props JSON, ...)` and `cl_parents`: every Drive
    item's metadata.
  - `cl_change_tokens(top_id, change_token)`: one changes-feed token per
    drive.
  - `acts` and `act_parts`: the activity feed. Kinds include upload,
    download, add, edit, move, rename, delete, share, link-share and ignore.
- `logs.db`: logs. The workers are a Supervisor, FileSystem (inotify, one
  watch per folder), GDCloudService, StatusService, GDBackupService,
  GDSearchService and MenuService (file manager menus). A unix socket
  server serves the `insync` CLI and the file manager plugins. The UI
  feeds are ActivityFeed, ProgressFeed, NoticeFeed and AccountsFeed.
  Per-folder work (`ChangeSyncedGDDir`, `MountTopGDItem`, `NodeUpdate`)
  runs with a semaphore limiting directory expansion.
- On disk:
  - `~/Insync/<email>/Google Drive/` holds My Drive. Shared with me and
    Shared drives get their own base folders.
  - Google-format files become link files: JSON
    `{"url", "file_id", "account_email"}` named `.gddoc`, `.gdsheet`,
    `.gdslides`, `.gddraw`, `.gdform`, `.gdmap`, `.gdscript`, `.gdnote`,
    `.gdtable`, or `.gdlink` for shortcuts. A MIME package and a hidden
    desktop entry open them in the browser.

canopy keeps the same split:
- a cloud mirror fed by the changes feed (`mirror.rs`)
- a base state per sync root (`db.rs`)
- a three-way planner (`engine.rs`)
- one daemon with a socket API (`daemon.rs`, `api.rs`)

canopy reads and writes the same link-file format, so an existing Insync
folder can be adopted without re-downloading (file matching).

## Insync features, and where canopy stands

✓ = in canopy's plan for v1, ○ = later, ✗ = deliberately not.

### Accounts
- ✓ Multiple Google accounts, each with its own base folder
  (`~/Canopy/<email>/My Drive`, `Shared with me`, `Shared drives/<name>`).
- ✓ Sign in through the browser. OAuth uses PKCE and a loopback redirect;
  the refresh token is kept in the Secret Service keyring (oo7).
  - ✓ "Alternative login": a pasted code, for headless machines.
- ✓ Re-authenticate when the token is revoked (Attention item).
- ✓ Remove account (keeps the files on disk).
- ✗ Licensing, trials, analytics.
- ✗ OneDrive and Dropbox (Google Drive only, for now).

### Base folders and locations
- ✓ Move the base folder: rename on disk, no re-download.
- ✓ A missing base folder (unmounted drive, renamed folder) pauses that
  root and shows "Locate…". It never deletes in the cloud.
- ✓ An empty base folder at start-up when files were synced: ask before
  syncing deletions.
- ✓ Offline deletions: setting "Sync deletions made while canopy wasn't
  running" (on), or "unsync instead" (dual boot).
- ✓ File matching. Adding an account over an existing folder (an Insync
  one included) matches by path, size and md5. Matching link files by
  `file_id` is ✓ too. Duplicates are not created.
- ✓ External and network drives as roots. A root whose disk is gone pauses
  until it comes back.

### Sync modes and selective sync
- ✓ Two-way sync (the default).
- ✓ Cloud → local, one-way: a mirror. Local edits are overwritten and local
  deletions re-downloaded.
- ✓ Local → cloud, one-way ("Sync any folder", backup). A setting chooses
  whether deletions on the computer are deleted in the cloud.
- ✓ Cloud Selective Sync: tick which cloud folders and files sync. The
  rest stay cloud-only and can be browsed in the Cloud Browser. Choices
  are kept as path rules, and the nearest rule wins.
- ✓ Local Selective Sync: any local folder outside the base folder synced
  to a new or existing cloud folder (merge or inside).
- ✓ Cloud items synced to a custom local location ("@").
- ✓ Unsync: the local copy goes to the system trash; the cloud copy is kept.
- ✓ Shared with me: its own base folder; items sync from where they live.
- ✓ Shared drives: one folder each.
- ✓ Starred: a Cloud Browser section, plus Star / Unstar.
- ✓ Drive shortcuts become `.gdlink` link files. ○ Follow shortcuts
  (Insync can't).
- ✓ Symlinks are not synced. One Attention item says so; it can be turned
  off.

### Google Docs, Sheets and Slides
- ✓ Link files (the default): double-click opens the browser.
- ✓ Convert to Microsoft Office (docx, xlsx, pptx) or OpenDocument
  (odt, ods, odp). Drawings export to SVG. Forms, scripts and maps stay
  as links.
  - Local edits upload back into the same Google file, which stays a
    Google file.
  - Exports over Drive's 10 MB limit fall back to the `exportLinks` URL.
- ✓ A link file copied locally becomes a copy in the cloud (`files.copy`).
- ✓ New Google Doc, Sheet, Slides or folder from the app.

### Ignore rules
- ✓ gitignore syntax, per account, relative to each root: bare names at any
  depth, `dir/path`, `*.ext`, `dir/*`, `!` exceptions and `\` escapes.
  Rules are case-sensitive. Uses the `ignore` crate's gitignore matcher.
- ✓ Built-in ignores, as in Insync:
  - names starting with `.~lock`, `~$`, `.~$`, `._`, `#` or `.trash-`
  - names ending with `.tmp`, `.swp`, `~`, `___jb_tmp___` or
    `___jb_old___`
  - names containing `(autosaved)`
  - `.Trash-*` folders
  - canopy's own `.canopy-part` folder
- ✓ Retroactive: a newly ignored synced item is unsynced locally, never
  deleted in the cloud.
- ✓ Ignored items listed in the Activity feed.

### Deletions and conflicts
- ✓ A cloud deletion moves the local copy to the freedesktop trash.
- ✓ A local deletion moves the cloud copy to Drive's trash.
- ✓ Conflicts: the four Insync cases, each set to "Ask" or a fixed
  choice, with a "Use this for all conflicts like this" button. The
  default is "keep both". The losing local copy is renamed
  `name (conflict <host> <date>).ext` and uploads as its own file.
- ✓ Duplicate names in one Drive folder: the oldest keeps the name; the
  others become `name (2).ext`, `name (3).ext`...
- ✓ Names Linux can't hold (`/`, `\0`) are escaped. Names Drive rejects
  become an Error item.

### App
- ✓ Home with feeds:
  - Activity: upload, download, add, edit, move, rename, delete, share,
    ignore. Filterable by account and kind.
  - Progress: transfers with progress, pause/resume, scanning.
  - Attention: errors with Retry, Dismiss, Retry all and Dismiss all.
  - Conflicts.
- ✓ Cloud Browser: My Drive, Shared with me, Shared drives and Starred,
  with selective-sync checkboxes.
  - Per-item: sync two-way or one-way, unsync, open location, view on the
    web, copy link, share, star, new.
- ✓ Search across accounts (the local mirror, plus Drive's full-text
  `fullText contains` when online).
- ✓ Share: copy link; "anyone with the link" on or off and its role;
  share with people by email, with a role and a notify message; list
  and remove access.
- ✓ Pause and resume, for everything and per account.
- ✓ Settings:
  - start at login (systemd user unit, or XDG autostart)
  - notifications
  - symlink warnings
  - offline-deletion handling
  - proxy (system, or manual HTTP/HTTPS/SOCKS5)
  - bandwidth limits up and down (Insync has none)
  - concurrent transfers (Insync is fixed at about 2)
- ✓ Tray icon (StatusNotifierItem, shown by hearth's bar). The icon shows
  the state: synced, syncing, paused, error or offline. The menu:
  - status line and current transfers
  - Pause/Resume
  - Open canopy
  - Open each base folder
  - recent activity
  - Quit
- ✓ Notifications through hearth's notification server (freedesktop
  Notifications D-Bus), throttled.
- ✓ The beginner-friendly rules apply: tooltips on every control, a Help
  window with searchable topics, first-run welcome.

### File manager integration (grove)
- ✓ Sync badges in grove's icon, list and column views. They come from
  canopy's socket, like grove's version-control badges, and refresh on
  push events. The states: synced ✓, syncing ↻, error ✕, conflict !,
  ignored –, and partly synced (a folder with cloud-only items).
- ✓ grove right-click "Google Drive" submenu:
  - Copy link
  - Share…
  - View on Drive
  - Sync / Unsync (Local Selective Sync of any folder)
  - Star
- ✓ `canopy open-cloud FILE` opens link files (MIME package
  `application/vnd.canopy.link.*` for `.gd*`, plus Insync's own types).

### Command line (Insync's headless CLI)
- ✓ `canopy status [-f PATH]`, `pause`, `resume`, `account add|list|remove`
  (`add --code` for headless sign-in), `ignore-rules list|add|clear`,
  `export-option`, `conflict list|resolution`,
  `error list|retry-all|dismiss-all`, `selective-sync` (a plain text
  tree, not ncurses), `open-cloud`, `share-link`, `config`, `quit`.
- ✓ `canopyd` runs with no display (a systemd user service). The tray and
  notifications are simply absent without a session bus.

### Transfers
- ✓ Resumable uploads (sessions are kept in the database for up to a week)
  and ranged downloads that resume, into `.canopy-part/`. Results are
  checked against md5, then atomically renamed. The cloud mtime is kept.
- ✓ Retries with exponential backoff on 429, 403 rate limits and 5xx.
- ✓ Bandwidth limits and transfer concurrency.
- ○ Download buffer memory limits are moot: canopy streams to disk.

## Engine

For each sync root (a cloud folder paired with a local folder, plus a mode
and selection rules), a pass:

1. Brings the mirror up to date: `changes.list` from the stored token,
   with `includeItemsFromAllDrives`, and one feed per shared drive.
2. Builds the cloud tree under the root from the mirror. Trashed items,
   unselected items and ignored names are left out. Duplicates get
   suffixes, and Google files get their local names.
3. Scans the local tree (the inotify watcher marks dirty folders; a full
   rescan runs at start and every few hours).
4. Detects moves before anything else:
   - Cloud moves are a known id at a new path.
   - Local moves are a known inode at a new path, with the same
     size and mtime for files.
   These become renames on the other side, so nothing is re-transferred.
5. Merges each path three ways (base, local, cloud):
   - Local changed: size or mtime differs from base.
   - Cloud changed: md5 differs (binary files) or version differs
     (Google files).
   - Changed on one side: copy it to the other side.
   - Changed on both sides with the same md5: record it as synced.
   - Otherwise it is a conflict, handled by the account's policy.
   - Folders are created top-down and deletions applied bottom-up.
   - A folder is deleted on the other side only when nothing inside it
     changed.
6. Applies each action and records the new base row right after it
   succeeds. An interrupted pass therefore resumes cleanly.

Safety:
- A missing root, a root on an unmounted filesystem, or an empty root that
  had files stops the pass with an Attention item.
- Deletions only ever go to a trash (the system trash locally, Drive's
  trash in the cloud).

Polling: `changes.list` every 20 s while active (5 units of quota each),
backing off when idle. Push notifications need a public HTTPS webhook, so a
desktop client can't use them.

## OAuth client

Google's `drive` scope is restricted. A public client needs Google's
verification and possibly a security assessment. canopy uses your own
Google Cloud OAuth client ("Desktop app" type), the same way rclone does:

- set the project's publishing status to "In production" (left unverified)
  so refresh tokens don't expire after 7 days
- add yourself as a user on the consent screen

Settings › Accounts › "Google Cloud client" holds the client ID and secret.
The secret of an installed app is not really secret. Help explains the
setup step by step.

## Processes

- `canopyd`: no Qt. Accounts, engines, watcher, changes polling, transfers,
  the socket API (`$XDG_RUNTIME_DIR/canopy/canopy.sock`, one JSON per
  line, request/response plus a `subscribe` event stream), the tray icon
  and notifications. Started by systemd (user unit) or by the window.
- `canopy`: the Qt Quick window, and the CLI (`canopy status`...). It only
  talks to the socket and never blocks the UI thread: replies come back as
  signals.
- grove: a socket client for badges and menu actions.
