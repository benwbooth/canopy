import QtQuick
import QtQuick.Controls.Basic

// Synced folders of the account: My Drive's folder, shared drives, and
// any folders synced from elsewhere, each with how it syncs.
Item {
    id: fv
    property var win
    readonly property var acct: win.acct
    readonly property string cl: win.cloudName(acct ? acct.id : "")
    readonly property var roots: acct ? acct.roots || [] : []
    readonly property string rootKey: roots.map(r => r.id).join("\n")
    readonly property var modes: [
        { text: "Two-Way", value: "two_way", tip: "Changes go both ways (the usual)" },
        { text: "Back Up", value: "local_to_cloud", tip: "Only this computer's changes go to " + fv.cl + "; the cloud copy follows this folder" },
        { text: "Download Only", value: "cloud_to_local", tip: "This folder follows " + fv.cl + "; changes made here aren't uploaded" }
    ]

    Item {
        id: head
        width: parent.width
        height: Math.round(52 * Theme.pt)
        GLabel { anchors.left: parent.left; anchors.leftMargin: 20; anchors.verticalCenter: parent.verticalCenter; text: "Synced Folders"; font.pixelSize: Theme.title; font.weight: Font.DemiBold }
        PushButton {
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            primary: true
            text: "Sync a Folder…"
            tip: Help.tip("Keep another folder on this computer in sync with " + fv.cl + " (or back it up)", "addfolder")
            onClicked: fv.win.act("addfolder")
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
        spacing: 10
        model: fv.rootKey ? fv.rootKey.split("\n") : []
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
        KineticWheel { flickable: list; notch: 120 }
        delegate: Rectangle {
            id: card
            required property var modelData
            readonly property var r: fv.roots.find(x => x.id === modelData) || ({ id: modelData, name: "", local: "", mode: "two_way", kind: "" })
            readonly property string held: ((fv.acct.state || {}).held || {})[r.id] || ""
            width: list.width
            height: col.implicitHeight + 24
            radius: Theme.glassRadius
            color: Theme.field
            Column {
                id: col
                x: 14; y: 12
                width: parent.width - 28
                spacing: 8
                Row {
                    width: parent.width
                    spacing: 10
                    GIcon { name: card.r.kind === "my_drive" ? "folder-cloud|folder-remote" : card.r.kind === "shared_drive" ? "network-server|folder-remote" : "folder-sync|folder"; size: 28; anchors.verticalCenter: parent.verticalCenter }
                    Column {
                        width: parent.width - 320
                        anchors.verticalCenter: parent.verticalCenter
                        GLabel { width: parent.width; text: card.r.name; font.weight: Font.DemiBold }
                        GLabel { width: parent.width; text: card.r.local; color: Theme.fgSecondary; font.pixelSize: Theme.small; elide: Text.ElideMiddle }
                    }
                    SyncBadge { sync: card.held ? "error" : fv.acct.paused ? "paused" : ((fv.acct.state || {}).phase === "syncing" ? "syncing" : "synced"); size: 18; anchors.verticalCenter: parent.verticalCenter; tip: card.held || "" }
                    GButton { text: "Review"; flat: false; iconName: "view-list-details|document-preview"; tip: "See what syncing this folder would do now (changes nothing)"; anchors.verticalCenter: parent.verticalCenter; onClicked: fv.win.openPreview(fv.acct.id, card.r.id) }
                    GButton { text: "Open"; flat: false; iconName: "folder-open"; tip: "Open this folder"; anchors.verticalCenter: parent.verticalCenter; onClicked: fv.win.cn.openPath(card.r.local) }
                }
                GLabel { visible: card.held !== ""; width: parent.width; text: card.held; color: Theme.danger; wrapMode: Text.Wrap; elide: Text.ElideNone }
                Row {
                    spacing: 10
                    GLabel { text: "Sync:"; color: Theme.fgSecondary; anchors.verticalCenter: parent.verticalCenter }
                    Segmented {
                        model: fv.modes
                        current: Math.max(0, fv.modes.findIndex(m => m.value === card.r.mode))
                        onPicked: i => fv.win.api("set_mode", { account: fv.acct.id, root: card.r.id, mode: fv.modes[i].value }, () => fv.win.toast.show("“" + card.r.name + "”: " + fv.modes[i].tip + "."))
                    }
                }
                Row {
                    spacing: 8
                    visible: card.r.kind === "custom"
                    GButton {
                        text: "Stop Syncing"
                        flat: false
                        tip: "Stop keeping this folder in sync. Both copies stay as they are."
                        onClicked: fv.win.confirmSheet("Stop syncing “" + card.r.name + "”?", "The folder on this computer and its copy in " + fv.cl + " both stay; they just won't be kept the same any more.", "Stop Syncing", () => fv.win.api("remove_root", { account: fv.acct.id, root: card.r.id }))
                    }
                    GButton {
                        visible: card.held.indexOf("missing") >= 0
                        text: "Locate…"
                        flat: false
                        tip: "Show canopy where the folder is now"
                        onClicked: Async.call(fv.win.cn, k => fv.win.cn.chooseFolder(k, card.r.local), res => { if (res && res.folder) fv.win.api("locate_root", { account: fv.acct.id, root: card.r.id, to: res.folder }) })
                    }
                }
            }
        }
    }
}
