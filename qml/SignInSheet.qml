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
    title: "Add a Google Account"
    width: Math.min(parent ? parent.width - 40 : 560, Math.round(520 * Theme.pt))
    onOpened: { waiting = false; pasting = false; error = ""; login = ""; url = "" }
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

    // Step 1: the client (once).
    Column {
        visible: si.win.status.has_client !== true && !si.waiting
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
        visible: !si.waiting
        width: parent.width
        wrapMode: Text.Wrap
        elide: Text.ElideNone
        text: "Your browser opens Google's sign-in. Choose your account; when Google says it hasn't verified the app, click Advanced › Go to your app, and allow everything it asks for. Your files go to ~/Canopy/<your email>."
    }
    Column {
        visible: si.waiting
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
        PushButton { visible: !si.waiting; primary: true; text: "Sign In with Google…"; enabled: si.hasClient; tip: si.hasClient ? "Open Google's sign-in in your browser" : "Paste your client ID and secret first"; onClicked: si.begin() }
    ]
}
