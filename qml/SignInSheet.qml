import QtQuick
import QtQuick.Controls.Basic

// Adding an account. Google: from hearth's Internet Accounts, or the
// one-time client setup and signing in through the browser (or pasting the
// address it ended on). Other clouds: a drive set up in rclone (hearth's
// Settings › Accounts › Cloud Drives sets them up; grove's Connect to Server
// without hearth), synced to its own folder.
Sheet {
    id: si
    property var win
    property int tab: 0                    // 0 Google, 1 other clouds
    readonly property bool google: tab === 0
    // rclone: {available, remotes: [{name, type, label, added}]}.
    property var remotes: ({ available: true, remotes: [] })
    property bool loadingRemotes: false
    function loadRemotes() {
        loadingRemotes = true
        win.api("rclone_remotes", {}, r => { si.remotes = r || { available: false, remotes: [] }; si.loadingRemotes = false }, e => { si.error = e; si.loadingRemotes = false })
    }
    // Set up a new drive: hearth's Settings › Accounts › Cloud Drives (grove
    // without hearth). The list here follows rclone's config meanwhile.
    function addCloudDrive() {
        if (win.status.hearth) win.api("hearth_open", { arg: "clouddrives:add" }, ok => { if (!ok) si.win.cn.openGrove() })
        else win.cn.openGrove()
    }
    Timer {
        interval: 3000
        repeat: true
        running: si.visible && !si.google && !si.waiting
        onTriggered: si.win.api("rclone_remotes", {}, r => { if (r && JSON.stringify(r) !== JSON.stringify(si.remotes)) si.remotes = r }, () => {})
    }
    function addRemote(r) {
        si.error = ""
        si.waiting = true
        const sub = folderField.text.trim().replace(/^\/+|\/+$/g, "")
        win.api("rclone_add", { remote: r.name + ":" + sub }, done => {
            si.waiting = false
            si.close()
            si.win.cn.refresh()
            si.win.toast.show("Syncing " + r.label + ". canopy is getting your files.")
        }, e => { si.waiting = false; si.error = e })
    }
    property string login: ""
    property string url: ""
    property bool waiting: false
    property bool pasting: false
    property string error: ""
    property string cid: ""
    property string csecret: ""
    readonly property bool hasClient: win.status.has_client === true || (cid.trim() !== "" && csecret.trim() !== "")
    // hearth's Internet Accounts: {available, registered, accounts: [...]}.
    property var hearth: ({ available: false })
    property bool own: false               // canopy's own sign-in instead
    readonly property bool useHearth: hearth.available === true && !own
    title: "Add an Account"
    width: Math.min(parent ? parent.width - 40 : 560, Math.round(520 * Theme.pt))
    onOpened: { waiting = false; pasting = false; error = ""; login = ""; url = ""; own = false; loadHearth(); loadRemotes() }
    function loadHearth() { win.api("hearth_accounts", {}, r => si.hearth = r || { available: false }, () => si.hearth = { available: false }) }
    // Internet Accounts changed (signed in there): list again.
    Connections {
        target: si.win.cn
        function onEvent(json) { if (si.visible && json.indexOf("hearth_changed") >= 0) si.loadHearth() }
    }
    function useAccount(a) {
        si.error = ""
        si.waiting = true
        win.api("hearth_link", { id: a.id }, done => {
            si.waiting = false
            si.close()
            si.win.cn.refresh()
            si.win.toast.show("Syncing " + done.email + ". canopy is getting your files.")
        }, e => { si.waiting = false; si.error = e })
    }
    onClosed: if (login && waiting) win.api("login_cancel", { login: login }, null, () => {})

    function begin() {
        error = ""
        const go = () => win.api("login_begin", {}, r => {
            si.login = r.login
            si.url = r.url
            si.waiting = true
            si.win.cn.openUrl(r.url)
            si.win.api("login_wait", { login: r.login }, done => {
                si.waiting = false
                si.close()
                si.win.cn.refresh()
                si.win.toast.show("Added " + done.email + ". canopy is getting your files.")
            }, e => { si.waiting = false; si.error = e })
        }, e => si.error = e)
        if (win.status.has_client !== true) win.api("set_settings", { settings: { client_id: cid.trim(), client_secret: csecret.trim() } }, go, e => si.error = e)
        else go()
    }

    Segmented {
        model: [{ text: "Google Drive", tip: "A Google account (Docs, shared drives, sharing)" }, { text: "Other Clouds", tip: "Dropbox, OneDrive, Box, S3, SFTP, WebDAV, pCloud… through rclone" }]
        current: si.tab
        enabled: !si.waiting
        onPicked: i => { si.tab = i; si.error = "" }
    }

    // ---- Other clouds (rclone) ----
    Column {
        visible: !si.google
        width: parent.width
        spacing: 10
        GLabel {
            width: parent.width
            wrapMode: Text.Wrap
            elide: Text.ElideNone
            color: Theme.fgSecondary
            text: si.remotes.available === false
                  ? "Other clouds go through rclone, which isn't installed."
                  : "canopy keeps a folder in ~/Canopy and the cloud the same, both ways (no mounting: the files are really here). " + ((si.remotes.remotes || []).length ? "Pick one of your cloud drives, or add another:" : "First add the cloud drive (once, for all your apps): click Add a Cloud Drive below, pick the service and sign in. It shows up here by itself.")
        }
        Repeater {
            model: si.remotes.remotes || []
            delegate: Rectangle {
                required property var modelData
                readonly property var r: modelData
                width: parent.width
                height: Math.round(46 * Theme.pt)
                radius: Theme.radius
                color: Theme.field
                GIcon {
                    id: ri
                    x: 12; anchors.verticalCenter: parent.verticalCenter
                    name: "folder-cloud|folder-remote"
                    size: 22
                }
                Column {
                    anchors.left: ri.right
                    anchors.leftMargin: 10
                    anchors.right: ra.left
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    GLabel { width: parent.width; text: r.label }
                    GLabel { width: parent.width; text: r.name + ":"; font.pixelSize: Theme.small; color: Theme.fgSecondary }
                }
                PushButton {
                    id: ra
                    anchors.right: parent.right
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !r.added
                    primary: true
                    enabled: !si.waiting
                    text: "Sync"
                    tip: "Keep " + r.label + " (" + r.name + ":" + (folderField.text.trim() ? folderField.text.trim() : "") + ") in sync with a folder in ~/Canopy"
                    onClicked: si.addRemote(r)
                }
                GLabel {
                    anchors.right: parent.right
                    anchors.rightMargin: 14
                    anchors.verticalCenter: parent.verticalCenter
                    visible: r.added
                    text: "Already syncing"
                    color: Theme.fgSecondary
                    font.pixelSize: Theme.small
                }
            }
        }
        GField {
            id: folderField
            visible: (si.remotes.remotes || []).length > 0
            width: parent.width
            placeholderText: "Only a folder of it (optional), e.g. Work or bucket/backup"
            tip: "Sync just this folder of the cloud instead of all of it (needed for S3 and other services where the top level holds buckets)"
        }
        Row {
            spacing: 8
            PushButton {
                primary: (si.remotes.remotes || []).length === 0
                text: "Add a Cloud Drive…"
                tip: si.win.status.hearth ? "Opens Settings › Accounts › Cloud Drives: pick the service (Dropbox, OneDrive, S3…) and sign in, once for all your apps" : "Opens grove's Connect to Server, where cloud drives are set up"
                onClicked: si.addCloudDrive()
            }
            GButton { flat: false; iconName: "view-refresh"; text: "Look Again"; tip: "List rclone's clouds again"; onClicked: si.loadRemotes() }
        }
        BusyDots { running: si.waiting || si.loadingRemotes; visible: si.waiting || si.loadingRemotes }
    }

    // ---- From hearth's Internet Accounts ----
    Column {
        visible: si.google && si.useHearth
        width: parent.width
        spacing: 10
        GLabel {
            width: parent.width
            wrapMode: Text.Wrap
            elide: Text.ElideNone
            color: Theme.fgSecondary
            text: "Google accounts come from Settings › Internet Accounts, where you set them up once for every app (mail, calendars, grove…). Pick one to sync:"
        }
        Repeater {
            model: si.hearth.accounts || []
            delegate: Rectangle {
                required property var modelData
                readonly property var a: modelData
                width: parent.width
                height: Math.round(52 * Theme.pt)
                radius: Theme.radius
                color: Theme.field
                Rectangle {
                    id: av
                    x: 10; anchors.verticalCenter: parent.verticalCenter
                    width: 30; height: 30; radius: 15; color: Theme.accent
                    GLabel { anchors.centerIn: parent; text: (a.email || "?").charAt(0).toUpperCase(); color: "white"; font.bold: true }
                }
                Column {
                    anchors.left: av.right
                    anchors.leftMargin: 10
                    anchors.right: act.left
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    GLabel { width: parent.width; text: a.email }
                    GLabel {
                        width: parent.width
                        font.pixelSize: Theme.small
                        color: a.can_sync && !a.signed_out ? Theme.fgSecondary : Theme.danger
                        text: a.linked ? "Already syncing" : a.signed_out ? "Signed out: sign in again" : a.can_sync ? "Ready to sync" : "Needs permission to see and change all your Drive files"
                    }
                }
                PushButton {
                    id: act
                    anchors.right: parent.right
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !a.linked
                    primary: a.can_sync && !a.signed_out
                    enabled: !si.waiting
                    text: a.can_sync && !a.signed_out ? (a.in_canopy ? "Use This" : "Sync") : "Sign In Again…"
                    tip: a.can_sync && !a.signed_out ? "Keep this account's Google Drive in sync with ~/Canopy/" + a.email
                         : "Opens Google in your browser: allow “See, edit, create and delete all of your Google Drive files” (canopy syncs every file)"
                    onClicked: a.can_sync && !a.signed_out ? si.useAccount(a) : si.win.api("hearth_signin", { id: a.id }, () => si.win.toast.show("Finish signing in in your browser; this list updates by itself."))
                }
            }
        }
        GLabel {
            visible: (si.hearth.accounts || []).length === 0
            width: parent.width
            wrapMode: Text.Wrap
            elide: Text.ElideNone
            text: si.hearth.registered ? "No Google account in Internet Accounts yet." : "Google sign-in isn't set up in Internet Accounts yet: Settings walks you through it once (about five minutes)."
        }
        Row {
            spacing: 8
            GButton {
                flat: false
                iconName: "list-add-user|list-add"
                text: si.hearth.registered ? "Add a Google Account in Settings…" : "Set Up Google in Settings…"
                tip: "Open Settings › Internet Accounts"
                onClicked: si.win.api("hearth_open", { arg: si.hearth.registered ? "signin:google" : "setup:google" })
            }
            GButton { flat: true; text: "Other Ways…"; tip: "Sign in with a Google Cloud client of canopy's own (not through Internet Accounts)"; onClicked: si.own = true }
        }
        BusyDots { running: si.waiting; visible: si.waiting }
    }

    // Step 1: the client (once).
    Column {
        visible: si.google && !si.useHearth && si.win.status.has_client !== true && !si.waiting
        width: parent.width
        spacing: 8
        GLabel {
            width: parent.width
            wrapMode: Text.Wrap
            elide: Text.ElideNone
            color: Theme.fgSecondary
            text: "First, once: Google only lets apps it has reviewed use your whole Drive, so canopy uses a sign-in client of your own. In the Google Cloud console make a project, enable the Google Drive API, set up the consent screen (External, publish it), and create an OAuth client of type “Desktop app”. Then paste its ID and secret here."
        }
        Row {
            spacing: 8
            GButton { text: "Open Google Cloud Console"; flat: false; iconName: "internet-web-browser"; tip: "console.cloud.google.com"; onClicked: si.win.cn.openUrl("https://console.cloud.google.com/apis/credentials") }
            GButton { text: "Step-by-Step Help"; flat: false; iconName: "help-contents"; tip: "Help › Setting up Google sign-in"; onClicked: si.win.act("help") }
        }
        GField { width: parent.width; placeholderText: "Client ID (….apps.googleusercontent.com)"; tip: "Credentials › your Desktop app client › Client ID"; onTextChanged: si.cid = text }
        GField { width: parent.width; placeholderText: "Client secret (GOCSPX-…)"; echoMode: TextInput.Password; tip: "Shown next to the client ID"; onTextChanged: si.csecret = text }
    }

    // Step 2: sign in.
    GLabel {
        visible: si.google && !si.useHearth && !si.waiting
        width: parent.width
        wrapMode: Text.Wrap
        elide: Text.ElideNone
        text: "Your browser opens Google's sign-in. Choose your account; when Google says it hasn't verified the app, click Advanced › Go to your app, and allow everything it asks for. Your files go to ~/Canopy/<your email>."
    }
    Column {
        visible: si.google && !si.useHearth && si.waiting
        width: parent.width
        spacing: 10
        Row {
            spacing: 10
            BusyDots { running: si.waiting; anchors.verticalCenter: parent.verticalCenter }
            GLabel { text: "Waiting for you to finish in the browser…" }
        }
        Row {
            spacing: 8
            GButton { text: "Open Again"; flat: false; tip: "Open the sign-in page in the browser again"; onClicked: si.win.cn.openUrl(si.url) }
            GButton { text: "Copy Address"; flat: false; tip: "Copy the sign-in address (to open it on another computer)"; onClicked: { si.win.cn.copyText(si.url); si.win.toast.show("Sign-in address copied.") } }
            GButton { text: "Paste the Answer…"; flat: false; tip: "Signed in on another computer? Paste the address its browser ended on"; onClicked: si.pasting = true }
        }
        GField {
            id: paste
            visible: si.pasting
            width: parent.width
            placeholderText: "http://127.0.0.1:…/?state=…&code=…"
            tip: "The address the browser showed after signing in (the page itself doesn't load)"
            onAccepted: si.win.api("login_paste", { login: si.login, text: text.trim() }, done => { si.waiting = false; si.close(); si.win.cn.refresh(); si.win.toast.show("Added " + done.email + ".") }, e => si.error = e)
        }
    }
    GLabel { visible: si.error !== ""; width: parent.width; text: si.error; color: Theme.danger; wrapMode: Text.Wrap; elide: Text.ElideNone }

    buttons: [
        PushButton { text: "Cancel"; onClicked: si.close() },
        PushButton { visible: si.google && !si.useHearth && !si.waiting; primary: true; text: "Sign In with Google…"; enabled: si.hasClient; tip: si.hasClient ? "Open Google's sign-in in your browser" : "Paste your client ID and secret first"; onClicked: si.begin() }
    ]
}
