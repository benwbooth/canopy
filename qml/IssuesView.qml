import QtQuick
import QtQuick.Controls.Basic

// Needs Attention (errors, held folders) or Conflicts, with what to do
// about each.
Item {
    id: iv
    property var win
    property bool conflicts: false
    property var rows: []
    readonly property var shown: rows.filter(r => conflicts ? r.kind === "conflict" : r.kind !== "conflict")
    // Folders canopy stopped (missing, emptied, mass delete): from status.
    readonly property var held: {
        const out = []
        if (conflicts) return out
        for (const a of win.accounts) {
            const h = (a.state || {}).held || {}
            for (const rid in h) {
                const root = (a.roots || []).find(r => r.id === rid) || {}
                out.push({ account: a.id, root: rid, name: root.name || rid, local: root.local || "", message: h[rid] })
            }
            if ((a.state || {}).phase === "signed_out") out.push({ account: a.id, root: "", name: a.email, signin: true, message: "Google access for " + a.email + " stopped working (signed out, or the access was removed)." })
        }
        return out
    }

    onVisibleChanged: if (visible) reload()
    function reload() { win.api("issues", {}, r => iv.rows = r || []) }
    function resolve(r, choice, always) {
        win.api("resolve", { account: r.account, root: r.root, path: r.path, choice: choice, always: always }, () => { iv.reload(); win.toast.show("Done: canopy syncs it now.") })
    }

    Item {
        id: head
        width: parent.width
        height: Math.round(52 * Theme.pt)
        GLabel { anchors.left: parent.left; anchors.leftMargin: 20; anchors.verticalCenter: parent.verticalCenter; text: iv.conflicts ? "Conflicts" : "Needs Attention"; font.pixelSize: Theme.title; font.weight: Font.DemiBold }
        Row {
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            spacing: 8
            visible: !iv.conflicts && iv.shown.length > 0
            GButton { text: "Retry All"; flat: false; tip: "Try every failed file again now"; onClicked: iv.win.api("retry", {}, () => iv.reload()) }
            GButton { text: "Dismiss All"; flat: false; tip: "Clear this list (canopy tries again on the next change)"; onClicked: iv.win.api("dismiss", {}, () => iv.reload()) }
        }
    }

    ListView {
        id: list
        anchors.top: head.bottom
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.leftMargin: 16
        anchors.rightMargin: 16
        clip: true
        spacing: 8
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
        header: Column {
            width: list.width
            spacing: 8
            Repeater {
                model: iv.held
                delegate: Rectangle {
                    required property var modelData
                    readonly property var h: modelData
                    width: list.width
                    height: hc.implicitHeight + 24
                    radius: Theme.glassRadius
                    color: Qt.rgba(Theme.danger.r, Theme.danger.g, Theme.danger.b, Theme.dark ? 0.16 : 0.08)
                    border.color: Qt.rgba(Theme.danger.r, Theme.danger.g, Theme.danger.b, 0.4)
                    Column {
                        id: hc
                        x: 14; y: 12
                        width: parent.width - 28
                        spacing: 8
                        GLabel { text: h.signin ? "Sign in again" : "Syncing “" + h.name + "” is paused"; font.weight: Font.DemiBold }
                        GLabel { width: parent.width; text: h.message; wrapMode: Text.Wrap; elide: Text.ElideNone; color: Theme.fgSecondary }
                        Row {
                            spacing: 8
                            PushButton { visible: h.signin === true; primary: true; text: "Sign In…"; onClicked: iv.win.act("addaccount") }
                            PushButton {
                                visible: !h.signin && h.message.indexOf("missing") >= 0
                                primary: true; text: "Locate…"
                                tip: "Show canopy where the folder is now"
                                onClicked: Async.call(iv.win.cn, k => iv.win.cn.chooseFolder(k, h.local), r => { if (r && r.folder) iv.win.api("locate_root", { account: h.account, root: h.root, to: r.folder }) })
                            }
                            PushButton {
                                visible: !h.signin && h.message.indexOf("first sync") >= 0
                                primary: true; text: "Start Syncing"
                                tip: "Go ahead: upload, download and keep both versions where they differ, as listed"
                                onClicked: iv.win.api("confirm", { account: h.account, root: h.root, what: "first_sync" })
                            }
                            GButton {
                                visible: !h.signin && h.root !== ""
                                flat: false; text: "Review…"
                                tip: "See every change syncing would make (nothing is done)"
                                onClicked: iv.win.openPreview(h.account, h.root)
                            }
                            PushButton {
                                visible: !h.signin && h.message.indexOf("is empty") >= 0
                                destructive: true; text: "Yes, Delete Them in Google Drive"
                                tip: "The files move to Google Drive's trash (kept 30 days)"
                                onClicked: iv.win.api("confirm", { account: h.account, root: h.root, what: "empty_root" })
                            }
                            PushButton {
                                visible: !h.signin && h.message.indexOf("would be deleted") >= 0
                                destructive: true; text: "Delete Them"
                                tip: "They move to Google Drive's trash (kept 30 days)"
                                onClicked: iv.win.api("confirm", { account: h.account, root: h.root, what: "mass_delete" })
                            }
                        }
                    }
                }
            }
            Item { width: 1; height: iv.held.length ? 4 : 0 }
        }
        model: iv.shown
        delegate: Rectangle {
            id: card
            required property var modelData
            readonly property var r: modelData
            width: list.width
            height: cc.implicitHeight + 24
            radius: Theme.glassRadius
            color: Theme.field
            Column {
                id: cc
                x: 14; y: 12
                width: parent.width - 28
                spacing: 6
                Row {
                    spacing: 8
                    width: parent.width
                    SyncBadge { sync: card.r.kind === "conflict" ? "C" : "E"; size: 16; anchors.verticalCenter: parent.verticalCenter }
                    GLabel { text: (card.r.path || card.r.root_name || "").split("/").pop() || card.r.root_name; font.weight: Font.DemiBold; width: parent.width - 30 }
                }
                GLabel { width: parent.width; text: card.r.message; wrapMode: Text.Wrap; elide: Text.ElideNone }
                GLabel { width: parent.width; text: card.r.local || card.r.path; color: Theme.fgTertiary; font.pixelSize: Theme.small; elide: Text.ElideMiddle }
                Row {
                    spacing: 8
                    visible: card.r.kind !== "conflict"
                    GButton { text: "Retry"; flat: false; tip: "Try this file again now"; onClicked: iv.win.api("retry", { account: card.r.account, id: card.r.id }, () => iv.reload()) }
                    GButton { text: "Dismiss"; flat: false; tip: "Remove it from this list"; onClicked: iv.win.api("dismiss", { account: card.r.account, id: card.r.id }, () => iv.reload()) }
                    GButton { visible: !!card.r.local; text: "Show"; flat: false; tip: "Show it in its folder"; onClicked: iv.win.cn.showInFiles(card.r.local) }
                }
                Column {
                    visible: card.r.kind === "conflict"
                    spacing: 8
                    width: parent.width
                    Row {
                        spacing: 8
                        PushButton { primary: true; text: "Keep Both"; tip: "This computer's version is renamed “(conflict …)” and both are kept everywhere"; onClicked: iv.resolve(card.r, "keep_both", always.checked) }
                        PushButton { text: card.r.sub === "local_delete_cloud_edit" ? "Delete It" : "Keep This Computer's"; tip: card.r.sub === "local_delete_cloud_edit" ? "Delete it in Google Drive too (it goes to Google Drive's trash)" : "Upload this computer's version over Google Drive's"; onClicked: iv.resolve(card.r, "keep_local", always.checked) }
                        PushButton { text: card.r.sub === "local_edit_cloud_delete" ? "Delete It" : "Keep Google Drive's"; tip: card.r.sub === "local_edit_cloud_delete" ? "Delete it here too (it goes to the Trash)" : "Download Google Drive's version; this computer's goes to the Trash"; onClicked: iv.resolve(card.r, "keep_cloud", always.checked) }
                    }
                    GCheck { id: always; text: "Do this for every conflict like this"; tip: "Remember this choice (Account settings › Conflicts)" }
                }
            }
        }
        GLabel {
            anchors.centerIn: parent
            visible: iv.shown.length === 0 && iv.held.length === 0
            text: iv.conflicts ? "No conflicts. When a file changes in two places, you'll choose here (or canopy keeps both, as set in Account settings)." : "Everything is syncing fine."
            color: Theme.fgSecondary
            width: Math.min(implicitWidth, list.width - 60)
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
        }
    }
}
