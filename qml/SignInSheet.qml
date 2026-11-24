import QtQuick
import QtQuick.Controls.Basic

// Adding a Google account: the one-time client setup if needed, then
// signing in through the browser (or pasting the address it ended on).
Sheet {
    id: si
    property var win
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
    title: "Add a Google Account"
    width: Math.min(parent ? parent.width - 40 : 560, Math.round(520 * Theme.pt))
    onOpened: { waiting = false; pasting = false; error = ""; login = ""; url = ""; own = false; loadHearth() }
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

    // ---- From hearth's Internet Accounts ----
    Column {
        visible: si.useHearth
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
        visible: !si.useHearth && si.win.status.has_client !== true && !si.waiting
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
        visible: !si.useHearth && !si.waiting
        width: parent.width
        wrapMode: Text.Wrap
        elide: Text.ElideNone
        text: "Your browser opens Google's sign-in. Choose your account; when Google says it hasn't verified the app, click Advanced › Go to your app, and allow everything it asks for. Your files go to ~/Canopy/<your email>."
    }
    Column {
        visible: !si.useHearth && si.waiting
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
        PushButton { visible: !si.useHearth && !si.waiting; primary: true; text: "Sign In with Google…"; enabled: si.hasClient; tip: si.hasClient ? "Open Google's sign-in in your browser" : "Paste your client ID and secret first"; onClicked: si.begin() }
    ]
}
