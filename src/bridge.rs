//! The Rust <-> QML boundary: the `Canopy` object the window talks to.
//!
//! Everything goes to canopyd over its socket on a background thread;
//! answers come back as `asyncReply(key, json)` (qml/Async.qml matches the
//! key to its callback) and the daemon's live events as `event(json)`.
//! Nothing here waits on the daemon, the disk or the network.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "C++" {
        include!("native_helpers.h");
        fn canopy_create_app();
        fn canopy_run(url: &QString, import_path: &QString) -> i32;
        fn canopy_set_icon_theme(dark: bool);
        fn canopy_grab_windows(path: &QString);
        fn canopy_copy_text(text: &QString);
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// canopyd's `status` answer (JSON), kept current.
        #[qproperty(QString, status_json, cxx_name = "statusJson")]
        /// Connected to canopyd.
        #[qproperty(bool, connected)]
        type Canopy = super::CanopyRust;
    }

    impl cxx_qt::Threading for Canopy {}

    #[auto_cxx_name]
    extern "RustQt" {
        /// The answer to a background call (key, JSON).
        #[qsignal]
        fn async_reply(self: Pin<&mut Canopy>, key: QString, json: QString);
        /// A live event from canopyd (transfer, activity, status...).
        #[qsignal]
        fn event(self: Pin<&mut Canopy>, json: QString);
        /// Another `canopy` was started: raise the window ({share?, add?}).
        #[qsignal]
        fn activate_requested(self: Pin<&mut Canopy>, json: QString);

        /// Connect to canopyd (starting it if needed) and follow it.
        #[qinvokable]
        fn start(self: Pin<&mut Canopy>);
        /// Any canopyd command (api.rs); answers {ok, result} or {ok: false, error}.
        #[qinvokable]
        fn call(self: &Canopy, key: &QString, cmd: &QString, args_json: &QString);
        /// Fetch the status again now.
        #[qinvokable]
        fn refresh(self: &Canopy);
        #[qinvokable]
        fn open_url(self: &Canopy, url: &QString);
        /// Open a folder or file in its app.
        #[qinvokable]
        fn open_path(self: &Canopy, path: &QString);
        /// Show a file selected in grove.
        #[qinvokable]
        fn show_in_files(self: &Canopy, path: &QString);
        /// The desktop's folder chooser. Answers {folder} or {cancelled} or {error}.
        #[qinvokable]
        fn choose_folder(self: &Canopy, key: &QString, current: &QString);
        #[qinvokable]
        fn copy_text(self: &Canopy, text: &QString);
        #[qinvokable]
        fn env(self: &Canopy, name: &QString) -> QString;
        #[qinvokable]
        fn set_icon_theme(self: &Canopy, dark: bool);
        /// Test hook: save the windows to `path`.
        #[qinvokable]
        fn grab_windows(self: &Canopy, path: &QString);
    }
}

pub use crate::controller::CanopyRust;
