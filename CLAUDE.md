# canopy conventions

Google Drive sync for Ben's desktop (arbor/hearth/grove), replacing Insync.
Rust + Qt Quick via cxx-qt, layout and nixpkgs pin copied from ~/src/sheaf
(and grove). DESIGN.md has Insync's feature list and the engine design.

- `core/` (crate canopy-core, no Qt):
  - `engine.rs`: the three-way sync of one root.
  - `mirror.rs`: Drive metadata from the changes feed.
  - `db.rs`: SQLite per account.
  - `drive.rs`: Drive v3 REST.
  - `auth.rs`: OAuth with PKCE and loopback; tokens in oo7.
  - `daemon.rs` and `handlers.rs`: canopyd and its socket commands.
  - `api.rs`: the protocol, plus a client.
  - `tray.rs`: SNI tray and notifications.
  - `watch.rs`: inotify.
  - `fake.rs`: an in-memory Drive.
  - `cargo test -p canopy-core`: engine scenarios run against the fake Drive.
- `src/`: the window (`bridge.rs`, `controller.rs`: one generic
  `call(key, cmd, args)` to canopyd), and `cli.rs` (Insync's headless
  commands: `canopy status`, ...).
- `qml/`: every file is registered automatically (build.rs). The shared
  components (G*, Sheet, AppMenu, GlobalMenu) are copies of sheaf/grove's.
- grove reads badges and menu actions from the same socket
  (grove src/canopy.rs, qml/CloudBadge.qml). Keep `folder_status`,
  `file_status`, `link`, `star`, `select` and the events compatible.

Rules (Ben's):
- Native text (native.cpp; build.rs rejects QtRendering/Curve), and the
  0.001 px nudge on the window shell and popups.
- Never block the UI thread: everything goes to canopyd off-thread.
- Tooltips on every control; help topics and shortcuts live in qml/Help.qml.
- Deletions only ever go to a trash. Tests use CANOPY_TRASH_DIR or
  Ctx.trash_dir, never the real trash.
- Build with `nix develop -c cargo build --workspace` (canopyd too), inside
  `systemd-run --user --scope -p MemoryMax=16G`.
- Commit and push every working piece (`git commit -- <paths>`).

Headless test (no Google account needed):
- Set scratch HOME/XDG dirs and `CANOPY_RUNTIME_DIR`, plus `CANOPY_DEMO=1`
  (a sample account on the fake Drive), `CANOPY_NO_KEYRING=1`,
  `CANOPY_NO_TRAY=1`, `CANOPY_TRASH_DIR`, `DBUS_SESSION_BUS_ADDRESS=disabled:`.
- Start `target/debug/canopyd`, then run `canopy --screenshot FILE` with
  `QT_QPA_PLATFORM=offscreen`.
- `CANOPY_TEST="page=browse:my_drive;folder=Documents;shot=x;sheet=settings;light"`
  runs steps first (Main.qml testTimer).
