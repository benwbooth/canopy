import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Window
import GlobalMenu
import Canopy

// canopy's window: the sidebar (activity, attention, conflicts, and each
// account's My Drive / Shared with me / Shared drives / Starred / synced
// folders), the page in the middle, transfers in a popover. Everything
// comes from canopyd; closing the window doesn't stop syncing.
ApplicationWindow {
    id: win
    // The toolbar is the title bar (the shared window chrome from
    // hearth/shared/app-menus, as in all of Ben's apps).
    flags: Qt.Window | Qt.FramelessWindowHint
    visible: true
    width: 1100
    height: 740
    minimumWidth: 760
    minimumHeight: 520
    title: "Canopy"
    color: Theme.window

    Canopy { id: canopy }
    // For the pages and sheets (files of their own can't see ids here).
    readonly property var cn: canopy
    readonly property alias toast: toast

    // ---- state ----
    readonly property var status: { try { return JSON.parse(canopy.statusJson || "{}") } catch (e) { return {} } }
    readonly property var accounts: status.accounts || []
    // Keys that change only when something structural does (string/bool
    // properties notify only on a real change): menus and list rows hang on
    // these, so a status update while syncing doesn't rebuild them.
    readonly property string accountKey: accounts.map(a => a.id).join("\n")
    readonly property var accountIds: accountKey ? accountKey.split("\n") : []
    readonly property bool hasAccounts: accountKey !== ""
    readonly property bool hasAcct: acct !== null
    readonly property string overall: status.state || (canopy.connected ? "" : "connecting")
    readonly property bool paused: status.paused === true
    property var settings: ({})
    property string page: "activity"          // activity, attention, conflicts, browse, folders, search
    property string account: ""               // the account of browse/folders pages
    property string section: "my_drive"       // my_drive, shared, shared_drives, starred
    property bool sidebarShown: true
    property var transfers: ({})              // "account/root/path" -> transfer
    readonly property int transferCount: Object.keys(transfers).length
    readonly property var acct: accounts.find(a => a.id === account) || accounts[0] || null
    readonly property int errorCount: accounts.reduce((n, a) => n + (a.errors || 0) + Object.keys((a.state || {}).held || {}).length, 0)
    readonly property int conflictCount: accounts.reduce((n, a) => n + (a.conflicts_count || 0), 0)
    readonly property bool anySheet: previewSheet.visible || settingsSheet.visible || accountSheet.visible || signIn.visible || shareSheet.visible || addFolder.visible || helpWindow.visible || confirm.visible || progress.visible

    Binding { target: Theme; property: "mode"; value: win.settings.appearance || "auto" }

    Component.onCompleted: {
        try { Theme.hearth = JSON.parse(canopy.env("CANOPY_HEARTH") || "{}") || {} } catch (e) {}
        Theme.wheelDebug = canopy.env("CANOPY_WHEEL_DEBUG") !== ""
        canopy.setIconTheme(Theme.dark)
        canopy.start()
        if (testing) testTimer.start()
    }
    Connections { target: Theme; function onDarkChanged() { canopy.setIconTheme(Theme.dark) } }

    // First status: settings, deep links (--share / --add-folder), welcome.
    property bool firstStatus: true
    onStatusChanged: {
        if (!firstStatus || !status.accounts) return
        firstStatus = false
        api("settings", {}, s => win.settings = s || {})
        if (!account && status.accounts.length) account = status.accounts[0].id
        const share = canopy.env("CANOPY_OPEN_SHARE")
        const add = canopy.env("CANOPY_OPEN_ADD")
        if (share) shareSheet.openFor({ path: share })
        else if (add) addFolder.openWith(add)
        else if (status.accounts.length === 0 && !testing) signIn.open()
    }

    // canopyd: call(cmd, args, done(result), failed(error)); errors go to a
    // toast unless `failed` handles them.
    function api(cmd, args, done, failed) {
        Async.call(canopy, k => canopy.call(k, cmd, JSON.stringify(args || {})), r => {
            if (r && r.ok) { if (done) done(r.result) }
            else if (failed) failed(r ? r.error : "canopy isn't answering")
            else toast.show(r ? r.error : "canopy isn't answering", [], true)
        })
    }

    Connections {
        target: canopy
        function onEvent(json) {
            const e = JSON.parse(json)
            if (e.event === "transfer") {
                const t = Object.assign({}, win.transfers)
                t[e.account + "/" + e.root + "/" + e.path] = e
                win.transfers = t
            } else if (e.event === "transfer_done") {
                const t = Object.assign({}, win.transfers)
                delete t[e.account + "/" + e.root + "/" + e.path]
                win.transfers = t
            }
            if (e.event === "activity" || e.event === "changed") reloadTimer.restart()
        }
    }
    // Pages reload a moment after changes (bursts make one reload).
    Timer { id: reloadTimer; interval: 600; onTriggered: pageStack.reload() }

    function go(p, acc, sec) {
        page = p
        if (acc !== undefined) account = acc
        if (sec !== undefined) section = sec
    }
    function confirmSheet(title, message, okText, run) { confirm.ask(title, message, okText, run) }
    function askName(title, def, run) { confirm.askName(title, def, run) }
    function openShare(target) { shareSheet.openFor(target) }
    function openPreview(acc, root) { previewSheet.openFor(acc, root) }
    function pauseResume() { api(paused ? "resume" : "pause", {}, () => canopy.refresh()) }
    function openBase() {
        const a = acct
        if (a && a.roots && a.roots.length) canopy.openPath(a.roots[0].local)
    }

    // ---- actions (menus, shortcuts) ----
    function act(id) {
        switch (id) {
        case "activity": go("activity"); break
        case "attention": go("attention"); break
        case "conflicts": go("conflicts"); break
        case "mydrive": go("browse", account, "my_drive"); break
        case "folders": go("folders"); break
        case "search": searchField.forceActiveFocus(); break
        case "back": if (page === "browse") pageStack.browseUp(); break
        case "pause": pauseResume(); break
        case "syncnow": api("sync_now", {}, () => toast.show("Looking for changes…")); break
        case "progress": progress.open(); break
        case "addfolder": addFolder.openWith(""); break
        case "openbase": openBase(); break
        case "addaccount": signIn.open(); break
        case "accountsettings": if (acct) accountSheet.openFor(acct.id); break
        case "settings": settingsSheet.open(); break
        case "sidebar": sidebarShown = !sidebarShown; break
        case "close": win.close(); break
        case "quit": api("quit", {}, () => Qt.quit(), () => Qt.quit()); break
        case "help": helpWindow.openAt(0); break
        case "shortcutlist": helpWindow.openAt(1); break
        case "about": toast.show("Canopy " + (status.version || "") + ": Google Drive sync for Linux. Open source (MIT)."); break
        }
    }
    // Same content as before (no reload, no rebuild).
    function same(a, b) { return JSON.stringify(a) === JSON.stringify(b) }
    function mk(id, text, icon, enabled) {
        return { id: id, text: text, icon: icon, shortcut: Help.key(id), enabled: enabled !== false }
    }
    readonly property var menuModel: [
        { title: "Canopy", items: [
            mk("about", "About Canopy", "help-about"),
            { separator: true },
            mk("settings", "Settings…", "preferences-system|configure"),
            mk("accountsettings", "Account Settings…", "user-identity|preferences-desktop-user", hasAcct),
            mk("addaccount", "Add Google Account…", "list-add-user|contact-new"),
            { separator: true },
            mk("close", "Close Window", "window-close"),
            mk("quit", "Quit Canopy", "application-exit")
        ] },
        { title: "Sync", items: [
            mk("pause", paused ? "Resume Syncing" : "Pause Syncing", paused ? "media-playback-start" : "media-playback-pause", hasAccounts),
            mk("syncnow", "Look for Changes Now", "view-refresh", hasAccounts && !paused),
            mk("progress", "Transfers…", "folder-download|download"),
            { separator: true },
            mk("addfolder", "Sync a Folder…", "folder-new|folder-add", hasAccounts),
            mk("openbase", "Open Google Drive Folder", "folder-open", hasAcct)
        ] },
        { title: "Go", items: [
            mk("activity", "Activity", "view-history|document-open-recent"),
            mk("attention", "Needs Attention", "dialog-warning"),
            mk("conflicts", "Conflicts", "dialog-question|vcs-conflicting"),
            { separator: true },
            mk("mydrive", "My Drive", "folder-cloud|folder-remote", hasAcct),
            mk("folders", "Synced Folders", "folder-sync|folder", hasAcct),
            mk("search", "Search", "edit-find|system-search", hasAccounts),
            { separator: true },
            mk("sidebar", sidebarShown ? "Hide Sidebar" : "Show Sidebar", "sidebar-show|view-left-pane")
        ] },
        { title: "Help", items: [
            mk("help", "Canopy Help", "help-contents"),
            mk("shortcutlist", "Keyboard Shortcuts", "input-keyboard|preferences-desktop-keyboard")
        ] }
    ]
    GlobalMenuBar { id: globalMenu; window: win; model: win.menuModel; onTriggered: id => win.act(id) }
    onVisibleChanged: if (visible) Qt.callLater(globalMenu.attach)
    AppMenu {
        id: appMenu
        model: win.menuModel
        onTriggered: id => win.act(id)
        accent: Theme.accent
        dark: Theme.dark
        popupType: Popup.Item
    }

    // ---- keyboard ----
    Repeater {
        model: ["activity", "attention", "conflicts", "mydrive", "folders", "search", "pause", "syncnow", "progress", "addfolder", "openbase",
                "accountsettings", "settings", "sidebar", "close", "quit", "help", "shortcutlist"]
        delegate: Item {
            required property string modelData
            Shortcut {
                sequences: (Help.entry(parent.modelData) || { keys: [] }).keys
                enabled: !win.anySheet
                onActivated: win.act(parent.modelData)
            }
        }
    }
    Shortcut { sequences: ["Alt+Up"]; enabled: !win.anySheet && win.page === "browse"; onActivated: pageStack.browseUp() }

    // ---- the window ----
    Item {
        id: shell
        anchors.fill: parent
        // A thousandth of a pixel: text never lands exactly on a half
        // device pixel at 130% (round letters dropped a pixel).
        anchors.topMargin: 0.001
        transform: Translate { x: 0.001 }

        Rectangle {
            id: toolbar
            width: parent.width
            height: Theme.toolbarHeight
            color: Theme.toolbar
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.separator }
            // Drag to move, double-click to maximize, right-click for the
            // window menu; minimize, maximize and close at the right end.
            ChromeTitleArea { anchors.fill: parent; window: win; menuDark: Theme.dark; menuAccent: Theme.accent }
            ChromeButtons {
                id: winButtons
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                window: win
                fg: Theme.fg
                bg: Theme.toolbar
            }

            Row {
                anchors.left: parent.left
                anchors.leftMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 10
                GButton {
                    iconName: "sidebar-show|view-left-pane|view-sidetree"
                    checked: win.sidebarShown
                    tip: Help.tip("Show or hide the sidebar", "sidebar")
                    onClicked: win.sidebarShown = !win.sidebarShown
                    anchors.verticalCenter: parent.verticalCenter
                }
                SyncBadge {
                    sync: win.overall
                    size: 22
                    anchors.verticalCenter: parent.verticalCenter
                }
                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    GLabel { text: win.headline(); font.weight: Font.DemiBold; width: Math.min(implicitWidth, 300) }
                    GLabel { text: win.subline(); font.pixelSize: Theme.caption; color: Theme.fgSecondary; width: Math.min(implicitWidth, 300) }
                }
            }
            GField {
                id: searchField
                anchors.centerIn: parent
                width: Math.min(Math.round(300 * Theme.pt), parent.width - 640)
                visible: win.accounts.length > 0 && width > 120
                iconName: "edit-find|system-search"
                placeholderText: "Search Google Drive"
                clearable: true
                tip: Help.tip("Find files by name or by the words in them, in every account", "search")
                onAccepted: if (text.trim()) win.go("search")
                onTextChanged: if (!text && win.page === "search") win.go("activity")
            }
            Row {
                anchors.right: winButtons.left
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                GButton {
                    id: progressButton
                    iconName: "folder-download|download|go-down"
                    text: win.transferCount > 0 ? String(win.transferCount) : ""
                    visible: win.accounts.length > 0
                    tip: Help.tip(win.transferCount ? win.transferCount + " transfer(s) in progress" : "Transfers (nothing moving right now)", "progress")
                    onClicked: progress.visible ? progress.close() : progress.open()
                }
                GButton {
                    text: win.paused ? "Resume" : "Pause"
                    iconName: win.paused ? "media-playback-start" : "media-playback-pause"
                    visible: win.accounts.length > 0
                    flat: false
                    tip: Help.tip(win.paused ? "Start syncing again" : "Stop syncing for now; changes are synced when you resume", "pause")
                    onClicked: win.pauseResume()
                }
                GButton {
                    id: menuButton
                    iconName: "overflow-menu-symbolic|view-more-symbolic|application-menu"
                    tip: "All menus (also in the menu bar at the top of the screen)"
                    onClicked: appMenu.openBelow(menuButton)
                }
                GButton {
                    iconName: "help-contents|help-about"
                    tip: Help.tip("Canopy Help", "help")
                    onClicked: helpWindow.openAt(0)
                }
            }
        }

        Sidebar {
            id: sidebar
            visible: win.sidebarShown
            anchors.top: toolbar.bottom
            anchors.bottom: parent.bottom
            width: visible ? Math.round(230 * Theme.pt) : 0
            win: win
            Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: Theme.separator }
        }

        PageStack {
            id: pageStack
            anchors.top: toolbar.bottom
            anchors.bottom: parent.bottom
            anchors.left: sidebar.right
            anchors.right: parent.right
            win: win
            searchText: searchField.text
        }

        Toast {
            id: toast
            anchors.horizontalCenter: pageStack.horizontalCenter
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 18
            z: 20
        }
    }

    function headline() {
        if (!canopy.connected) return "Starting canopy…"
        switch (overall) {
        case "syncing": return transferCount ? "Syncing " + transferCount + " file" + (transferCount === 1 ? "" : "s") + "…" : "Syncing…"
        case "paused": return "Paused"
        case "offline": return "Offline"
        case "error": return "Needs attention"
        case "signed_out": return "Sign in again"
        case "empty": return "No account yet"
        default: return "Up to date"
        }
    }
    function subline() {
        if (!canopy.connected) return "Connecting to the sync service"
        if (overall === "empty") return "Add a Google account to start"
        if (overall === "offline") return "Waiting for the network; nothing is lost"
        if (overall === "signed_out") return "Google access stopped: sign in again"
        if (overall === "error") return (errorCount + conflictCount) + " item(s) to look at"
        const last = accounts.reduce((m, a) => Math.max(m, (a.state || {}).last_sync || 0), 0)
        return last ? "Checked " + ago(last) : accounts.map(a => a.email).join(", ")
    }
    function ago(t) {
        const s = Date.now() / 1000 - t
        if (s < 60) return "just now"
        if (s < 3600) return Math.round(s / 60) + " min ago"
        if (s < 86400) return Math.round(s / 3600) + " h ago"
        return new Date(t * 1000).toLocaleDateString()
    }
    function size(n) {
        if (n === null || n === undefined) return ""
        if (n < 1000) return n + " bytes"
        const u = ["KB", "MB", "GB", "TB"]
        let i = -1
        do { n /= 1000; i++ } while (n >= 1000 && i < u.length - 1)
        return (n < 10 ? n.toFixed(1) : Math.round(n)) + " " + u[i]
    }

    // ---- sheets ----
    ProgressPopup { id: progress; win: win; anchorItem: progressButton }
    SettingsSheet { id: settingsSheet; win: win }
    AccountSheet { id: accountSheet; win: win }
    SignInSheet { id: signIn; win: win }
    ShareSheet { id: shareSheet; win: win }
    AddFolderSheet { id: addFolder; win: win }
    ConfirmSheet { id: confirm }
    PreviewSheet { id: previewSheet; win: win }
    HelpWindow { id: helpWindow }

    // ---- test hook: CANOPY_TEST steps, then --screenshot ----
    readonly property string testSpec: canopy.env("CANOPY_TEST")
    readonly property string shotPath: canopy.env("CANOPY_SCREENSHOT")
    readonly property bool testing: testSpec !== "" || shotPath !== ""
    property var testSteps: testSpec ? testSpec.split(";").filter(s => s) : []
    property int testWait: 8
    Timer {
        id: testTimer
        interval: 250
        repeat: true
        onTriggered: {
            if (win.testWait > 0) { win.testWait--; return }
            if (!canopy.connected) return
            if (!win.testSteps.length) {
                stop()
                if (win.shotPath) canopy.grabWindows(win.shotPath)
                Qt.quit()
                return
            }
            const s = win.testSteps.shift()
            const cmd = s.split("=")[0], arg = s.split("=").slice(1).join("=")
            console.log("canopy test:", s)
            win.testWait = 4
            switch (cmd) {
            case "page": { const p = arg.split(":"); win.go(p[0], undefined, p[1]); break }
            case "folder": pageStack.openFolderNamed(arg); break
            case "search": searchField.text = arg; win.go("search"); break
            case "wait": win.testWait = Number(arg || 8); break
            case "shot": canopy.grabWindows(win.shotPath.replace(".png", "-" + arg + ".png")); break
            case "light": win.settings = Object.assign({}, win.settings, { appearance: "light" }); break
            case "dark": win.settings = Object.assign({}, win.settings, { appearance: "dark" }); break
            case "sheet": {
                const m = { settings: settingsSheet, signin: signIn, help: helpWindow, progress: progress, add: addFolder }
                if (arg === "account") accountSheet.openFor(win.acct.id)
                else if (arg === "share") shareSheet.openFor({ account: win.acct.id, id: pageStack.firstItemId() })
                else m[arg].open()
                break
            }
            case "newmenu": pageStack.openNewMenu(); break
            case "review": win.openPreview(win.acct.id, ""); win.testWait = 10; break
            case "close": for (const sh of [previewSheet, settingsSheet, accountSheet, signIn, shareSheet, addFolder, helpWindow, progress]) sh.close(); break
            }
        }
    }

    // Resizing from the edges (no system frame).
    ChromeResizeEdges { anchors.fill: parent; z: 10000; window: win }
}
