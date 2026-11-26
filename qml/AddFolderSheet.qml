import QtQuick
import QtQuick.Controls.Basic

// Sync any folder on this computer with an account's cloud (Insync's Local
// Selective Sync), two-way, as a backup, or download-only.
Sheet {
    id: af
    property var win
    property string folder: ""
    property int mode: 0
    property int acctIndex: 0
    readonly property string cl: win.accounts.length ? win.cloudName(win.accounts[Math.min(acctIndex, win.accounts.length - 1)].id) : "Google Drive"
    property string error: ""
    readonly property var modes: [
        { text: "Two-Way", value: "two_way", tip: "Changes go both ways" },
        { text: "Back Up", value: "local_to_cloud", tip: "Only this computer's changes go up; " + af.cl + " keeps a copy" },
        { text: "Download Only", value: "cloud_to_local", tip: "The folder follows " + af.cl }
    ]
    title: "Sync a Folder with " + cl
    width: Math.min(parent ? parent.width - 40 : 560, Math.round(520 * Theme.pt))
    function openWith(p) { folder = p || ""; error = ""; mode = 0; open(); if (!folder) choose() }
    function choose() { Async.call(win.cn, k => win.cn.chooseFolder(k, folder), r => { if (r && r.folder) af.folder = r.folder; else if (r && r.error) af.error = r.error }) }

    GLabel {
        width: parent.width
        wrapMode: Text.Wrap
        elide: Text.ElideNone
        color: Theme.fgSecondary
        text: "canopy makes a folder with the same name at the top of " + (af.cl === "Google Drive" ? "your Google Drive" : af.cl) + " (or uses one that's already there) and keeps the two the same."
    }
    Row {
        width: parent.width
        spacing: 8
        GField { width: parent.width - pick.width - 8; text: af.folder; placeholderText: "Choose a folder…"; tip: "The folder on this computer"; onEditingFinished: af.folder = text.trim() }
        GButton { id: pick; text: "Choose…"; flat: false; tip: "Pick the folder"; onClicked: af.choose() }
    }
    Row {
        spacing: 10
        visible: af.win.accounts.length > 1
        GLabel { text: "Account:"; anchors.verticalCenter: parent.verticalCenter }
        GCombo { model: af.win.accounts.map(a => af.win.accountLabel(a)); currentIndex: af.acctIndex; tip: "Which account"; onActivated: i => af.acctIndex = i }
    }
    Row {
        spacing: 10
        GLabel { text: "Sync:"; anchors.verticalCenter: parent.verticalCenter }
        Segmented { model: af.modes; current: af.mode; onPicked: i => af.mode = i }
    }
    GLabel { width: parent.width; text: af.modes[af.mode].tip + "."; color: Theme.fgSecondary; font.pixelSize: Theme.small; wrapMode: Text.Wrap; elide: Text.ElideNone }
    GLabel { visible: af.error !== ""; width: parent.width; text: af.error; color: Theme.danger; wrapMode: Text.Wrap; elide: Text.ElideNone }
    buttons: [
        PushButton { text: "Cancel"; onClicked: af.close() },
        PushButton {
            primary: true
            text: "Sync"
            enabled: af.folder !== "" && af.win.accounts.length > 0
            onClicked: af.win.api("add_folder", { account: af.win.accounts[af.acctIndex].id, local: af.folder, mode: af.modes[af.mode].value }, () => {
                af.close()
                af.win.go("folders", af.win.accounts[af.acctIndex].id)
                af.win.toast.show("“" + af.folder.split("/").pop() + "” is now synced with " + af.cl + ".")
            }, e => af.error = e)
        }
    ]
}
