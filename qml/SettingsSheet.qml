import QtQuick
import QtQuick.Controls.Basic

// canopy's settings: general, network, Google sign-in. Every change is
// saved at once (no Apply).
Sheet {
    id: ss
    property var win
    property int tab: 0
    property var s: ({})
    title: "Settings"
    width: Math.min(parent ? parent.width - 40 : 600, Math.round(560 * Theme.pt))
    onOpened: win.api("settings", {}, r => ss.s = r || {})
    function put(k, v) {
        const o = {}
        o[k] = v
        s = Object.assign({}, s, o)
        win.settings = Object.assign({}, win.settings, o)
        win.api("set_settings", { settings: o })
    }

    component Line: Item {
        property string label: ""
        property string help: ""
        default property alias control: holder.data
        width: parent.width
        height: Math.max(lc.implicitHeight, holder.childrenRect.height) + 6
        Column {
            id: lc
            width: parent.width - holder.width - 16
            GLabel { width: parent.width; text: parent.parent.label; wrapMode: Text.Wrap; elide: Text.ElideNone }
            GLabel { width: parent.width; visible: text !== ""; text: parent.parent.help; font.pixelSize: Theme.small; color: Theme.fgSecondary; wrapMode: Text.Wrap; elide: Text.ElideNone }
        }
        Item { id: holder; anchors.right: parent.right; width: childrenRect.width; height: childrenRect.height }
    }

    Segmented {
        model: [{ text: "General", tip: "Start-up, notifications, appearance" }, { text: "Network", tip: "Speed limits and proxy" }, { text: "Google Sign-In", tip: "Your Google Cloud client (needed to add accounts)" }]
        current: ss.tab
        onPicked: i => ss.tab = i
    }

    Column {
        visible: ss.tab === 0
        width: parent.width
        spacing: 10
        Line { label: "Start canopy when you log in"; help: "Syncing starts in the background (the window stays closed)."; GSwitch { checked: ss.s.start_at_login !== false; tip: "Start syncing at login"; onToggled: ss.put("start_at_login", checked) } }
        Line { label: "Notifications"; help: "Tell me about conflicts, errors and paused folders."; GSwitch { checked: ss.s.notifications !== false; tip: "Show notifications"; onToggled: ss.put("notifications", checked) } }
        Line { label: "Warn about symbolic links"; help: "Links can't be stored in Google Drive; canopy lists them once under Needs Attention."; GSwitch { checked: ss.s.symlink_warnings !== false; tip: "Show symbolic link warnings"; onToggled: ss.put("symlink_warnings", checked) } }
        Line {
            label: "Deletions made while canopy wasn't running"
            help: "Delete in Google Drive too, or only stop syncing those files (safer when the folder is shared with another system, e.g. dual boot)."
            GCombo {
                model: ["Delete in Google Drive", "Only stop syncing them"]
                currentIndex: ss.s.sync_offline_deletions === false ? 1 : 0
                tip: "What a deletion made while canopy was off means"
                onActivated: i => ss.put("sync_offline_deletions", i === 0)
            }
        }
        Line {
            label: "Appearance"
            help: "Follows hearth's light and dark setting unless you choose."
            Segmented {
                model: [{ text: "Auto", tip: "Like the desktop" }, { text: "Light", tip: "Always light" }, { text: "Dark", tip: "Always dark" }]
                current: ["auto", "light", "dark"].indexOf(ss.s.appearance || "auto")
                onPicked: i => ss.put("appearance", ["auto", "light", "dark"][i])
            }
        }
    }

    Column {
        visible: ss.tab === 1
        width: parent.width
        spacing: 10
        Line {
            label: "Upload speed limit"
            help: "KB per second; 0 means no limit."
            GField { width: 110; text: String(ss.s.upload_limit || 0); inputMethodHints: Qt.ImhDigitsOnly; tip: "Limit uploads (KB/s, 0 = none)"; onEditingFinished: ss.put("upload_limit", Math.max(0, parseInt(text) || 0)) }
        }
        Line {
            label: "Download speed limit"
            help: "KB per second; 0 means no limit."
            GField { width: 110; text: String(ss.s.download_limit || 0); inputMethodHints: Qt.ImhDigitsOnly; tip: "Limit downloads (KB/s, 0 = none)"; onEditingFinished: ss.put("download_limit", Math.max(0, parseInt(text) || 0)) }
        }
        Line {
            label: "Files transferred at once"
            help: "More is faster on a fast connection; Google may slow down very many small files."
            GCombo { model: ["1", "2", "4", "6", "8", "12", "16"]; currentIndex: Math.max(0, model.indexOf(String(ss.s.transfers || 4))); tip: "Parallel transfers"; onActivated: i => ss.put("transfers", parseInt(model[i])) }
        }
        Line {
            label: "Check Google Drive for changes every"
            help: "How quickly changes made elsewhere show up here."
            GCombo { model: ["10 seconds", "20 seconds", "1 minute", "5 minutes"]; readonly property var secs: [10, 20, 60, 300]; currentIndex: Math.max(0, secs.indexOf(ss.s.poll_seconds || 20)); tip: "Polling interval"; onActivated: i => ss.put("poll_seconds", secs[i]) }
        }
        Line {
            label: "Proxy"
            help: "System uses the desktop's proxy settings (http_proxy...)."
            GCombo {
                model: ["System", "None", "Manual"]
                readonly property var vals: ["system", "none", "manual"]
                currentIndex: Math.max(0, vals.indexOf((ss.s.proxy || {}).mode || "system"))
                tip: "How to reach the internet"
                onActivated: i => ss.put("proxy", { mode: vals[i], url: (ss.s.proxy || {}).url || "" })
            }
        }
        GField {
            visible: (ss.s.proxy || {}).mode === "manual"
            width: parent.width
            placeholderText: "socks5://host:1080 or http://user:password@host:3128"
            text: (ss.s.proxy || {}).url || ""
            tip: "The proxy's address"
            onEditingFinished: ss.put("proxy", { mode: "manual", url: text.trim() })
        }
    }

    Column {
        visible: ss.tab === 2
        width: parent.width
        spacing: 10
        GLabel {
            width: parent.width
            wrapMode: Text.Wrap
            elide: Text.ElideNone
            color: Theme.fgSecondary
            text: "canopy signs in to Google through an OAuth client of your own (Google only lets apps it has reviewed use whole Drives). Make a \"Desktop app\" client once in the Google Cloud console and paste it here. Help › Setting up Google sign-in has the steps."
        }
        Row {
            spacing: 8
            GButton { text: "Open Google Cloud Console"; flat: false; iconName: "internet-web-browser"; tip: "console.cloud.google.com › APIs & Services › Credentials"; onClicked: ss.win.cn.openUrl("https://console.cloud.google.com/apis/credentials") }
            GButton { text: "Show the Steps"; flat: false; iconName: "help-contents"; tip: "Help: setting up Google sign-in"; onClicked: { ss.close(); ss.win.act("help") } }
        }
        GLabel { text: "Client ID" }
        GField { width: parent.width; text: ss.s.client_id || ""; placeholderText: "1234567890-abc….apps.googleusercontent.com"; tip: "From Credentials › OAuth 2.0 Client IDs"; onEditingFinished: if (text.trim() !== (ss.s.client_id || "")) ss.put("client_id", text.trim()) }
        GLabel { text: "Client secret" }
        GField { width: parent.width; text: ss.s.client_secret || ""; echoMode: TextInput.Password; placeholderText: "GOCSPX-…"; tip: "Shown next to the client ID (for desktop apps it isn't really secret)"; onEditingFinished: if (text.trim() !== (ss.s.client_secret || "")) ss.put("client_secret", text.trim()) }
    }

    buttons: [ PushButton { primary: true; text: "Done"; onClicked: ss.close() } ]
}
