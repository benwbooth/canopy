import QtQuick
import QtQuick.Controls.Basic

// Share a file or folder: its link, who can open it with the link, and
// the people it's shared with.
Sheet {
    id: sh
    property var win
    property var target: ({})            // {path} or {account, id}
    property var item: null
    property var perms: []
    property bool busy: false
    property string error: ""
    readonly property var anyone: perms.find(p => p.type === "anyone") || null
    readonly property var people: perms.filter(p => p.type !== "anyone")
    readonly property var roles: [{ v: "reader", t: "Viewer" }, { v: "commenter", t: "Commenter" }, { v: "writer", t: "Editor" }]
    title: item ? "Share “" + item.name + "”" : "Share"
    width: Math.min(parent ? parent.width - 40 : 600, Math.round(540 * Theme.pt))

    function openFor(t) {
        target = t; item = null; perms = []; error = ""
        open()
        load()
    }
    function load() {
        busy = true
        win.api("share_info", target, r => { sh.item = r.item; sh.perms = r.permissions || []; sh.busy = false }, e => { sh.error = e; sh.busy = false })
    }
    function act(o) {
        busy = true
        win.api("share", Object.assign({}, target, o), r => { sh.perms = r.permissions || []; sh.busy = false }, e => { sh.error = e; sh.busy = false })
    }
    function roleIndex(r) { return Math.max(0, roles.findIndex(x => x.v === r)) }

    BusyDots { running: sh.busy; visible: sh.busy && !sh.item }

    Column {
        visible: sh.item !== null
        width: parent.width
        spacing: 12

        // Link.
        Row {
            width: parent.width
            spacing: 8
            GField { id: link; width: parent.width - copy.width - 8; readOnly: true; text: sh.item ? sh.item.web : ""; tip: "This file's address on Google Drive" }
            PushButton { id: copy; text: "Copy Link"; tip: "Copy the address"; onClicked: { sh.win.cn.copyText(link.text); sh.win.toast.show("Link copied.") } }
        }

        // General access.
        GLabel { text: "General access"; font.weight: Font.DemiBold }
        Row {
            spacing: 8
            GCombo {
                width: Math.round(220 * Theme.pt)
                model: ["Only people added below", "Anyone with the link"]
                currentIndex: sh.anyone ? 1 : 0
                tip: "Who can open it with just the link"
                onActivated: i => sh.act({ action: "anyone", role: i === 1 ? "reader" : "" })
            }
            GCombo {
                visible: sh.anyone !== null
                width: Math.round(130 * Theme.pt)
                model: sh.roles.map(r => r.t)
                currentIndex: sh.anyone ? sh.roleIndex(sh.anyone.role) : 0
                tip: "What people with the link can do"
                onActivated: i => sh.act({ action: "anyone", role: sh.roles[i].v })
            }
        }

        // People.
        GLabel { text: "People with access"; font.weight: Font.DemiBold }
        Repeater {
            model: sh.people
            delegate: Row {
                required property var modelData
                readonly property var p: modelData
                width: parent.width
                spacing: 8
                Rectangle {
                    width: 28; height: 28; radius: 14; color: Theme.accent
                    anchors.verticalCenter: parent.verticalCenter
                    GLabel { anchors.centerIn: parent; text: (p.displayName || p.emailAddress || "?").charAt(0).toUpperCase(); color: "white"; font.bold: true }
                }
                Column {
                    width: parent.width - 28 - 150 - 40 - 24
                    anchors.verticalCenter: parent.verticalCenter
                    GLabel { width: parent.width; text: p.displayName || p.emailAddress || p.domain || "" }
                    GLabel { width: parent.width; text: p.emailAddress || ""; color: Theme.fgSecondary; font.pixelSize: Theme.small }
                }
                GLabel { visible: p.role === "owner"; width: 150; text: "Owner"; color: Theme.fgSecondary; anchors.verticalCenter: parent.verticalCenter }
                GCombo {
                    visible: p.role !== "owner"
                    width: 150
                    model: sh.roles.map(r => r.t)
                    currentIndex: sh.roleIndex(p.role)
                    tip: "What this person can do"
                    anchors.verticalCenter: parent.verticalCenter
                    onActivated: i => sh.act({ action: "role", perm: p.id, role: sh.roles[i].v })
                }
                GButton {
                    visible: p.role !== "owner"
                    iconName: "list-remove|edit-delete"
                    tip: "Remove their access"
                    anchors.verticalCenter: parent.verticalCenter
                    onClicked: sh.act({ action: "remove", perm: p.id })
                }
            }
        }

        // Add people.
        GLabel { text: "Add people"; font.weight: Font.DemiBold }
        Row {
            width: parent.width
            spacing: 8
            GField { id: email; width: parent.width - 150 - 8; placeholderText: "name@example.com"; tip: "Their Google account's email address" }
            GCombo { id: newRole; width: 150; model: sh.roles.map(r => r.t); currentIndex: 0; tip: "What they can do" }
        }
        GField { id: msg; width: parent.width; placeholderText: "Message (optional)"; tip: "Sent with the email Google sends them" }
        Row {
            spacing: 12
            GCheck { id: notify; checked: true; text: "Notify them by email"; tip: "Google emails them a link" }
            PushButton {
                primary: true
                text: "Share"
                enabled: /.+@.+\..+/.test(email.text.trim()) && !sh.busy
                onClicked: { sh.act({ action: "add", email: email.text.trim(), role: sh.roles[newRole.currentIndex].v, notify: notify.checked, message: msg.text }); email.text = ""; msg.text = "" }
            }
        }
    }
    GLabel { visible: sh.error !== ""; width: parent.width; text: sh.error; color: Theme.danger; wrapMode: Text.Wrap; elide: Text.ElideNone }
    buttons: [ PushButton { primary: true; text: "Done"; onClicked: sh.close() } ]
}
