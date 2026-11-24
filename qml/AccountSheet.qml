import QtQuick
import QtQuick.Controls.Basic

// One account's settings: Google Docs, conflicts, ignore rules, its
// folder, shared drives, pausing, removing.
Sheet {
    id: as_
    property var win
    property string accountId: ""
    property int tab: 0
    readonly property var a: win.accounts.find(x => x.id === accountId) || ({})
    title: a.email || "Account"
    width: Math.min(parent ? parent.width - 40 : 640, Math.round(600 * Theme.pt))
    function openFor(id) { accountId = id; tab = 0; rules.text = (a.ignore_rules || []).join("\n"); open() }
    function put(o) { win.api("set_account", Object.assign({ account: accountId }, o), () => win.cn.refresh()) }
    readonly property var resolutions: [
        { v: "keep_both", t: "Keep both (rename this computer's)" },
        { v: "ask", t: "Ask me" },
        { v: "keep_local", t: "Keep this computer's" },
        { v: "keep_cloud", t: "Keep Google Drive's" }
    ]

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
        model: [{ text: "General", tip: "Folder, pausing, shared drives" }, { text: "Google Docs", tip: "How Docs, Sheets and Slides appear on this computer" }, { text: "Conflicts", tip: "What to do when a file changes in two places" }, { text: "Ignore Rules", tip: "Files canopy leaves alone" }]
        current: as_.tab
        onPicked: i => as_.tab = i
    }

    Column {
        visible: as_.tab === 0
        width: parent.width
        spacing: 10
        Line {
            label: "Folder"
            help: as_.a.base || ""
            Row {
                spacing: 6
                GButton { text: "Open"; flat: false; tip: "Open this account's folder"; onClicked: as_.win.cn.openPath(as_.a.base) }
                GButton {
                    text: "Move…"
                    flat: false
                    tip: "Move the folder somewhere else (another disk too); nothing is downloaded again"
                    onClicked: Async.call(as_.win.cn, k => as_.win.cn.chooseFolder(k, as_.a.base), r => {
                        if (r && r.folder) as_.win.api("move_base", { account: as_.accountId, to: r.folder + "/" + as_.a.email }, () => as_.win.toast.show("Moved to " + r.folder + "/" + as_.a.email + "."))
                    })
                }
            }
        }
        Line { label: "Pause this account"; help: "Stop syncing just this account."; GSwitch { checked: as_.a.paused === true; tip: "Pause or resume this account"; onToggled: as_.put({ paused: checked }) } }
        Line { label: "Sync shared drives"; help: "Each shared drive gets a folder inside “Shared drives”."; GSwitch { checked: as_.a.sync_shared_drives === true; tip: "Sync your team's shared drives too"; onToggled: as_.put({ sync_shared_drives: checked }) } }
        Line {
            label: "Storage"
            help: ((as_.a.state || {}).quota_total ? as_.win.size((as_.a.state || {}).quota_used) + " of " + as_.win.size((as_.a.state || {}).quota_total) + " used" : "")
            GButton { text: "Manage on Google"; flat: false; tip: "one.google.com/storage"; onClicked: as_.win.cn.openUrl("https://one.google.com/storage") }
        }
        Line {
            label: "Remove this account"
            help: "canopy stops syncing it and signs out. Your files stay in the folder and in Google Drive."
            PushButton {
                destructive: true
                text: "Remove…"
                onClicked: as_.win.confirmSheet("Remove " + as_.a.email + "?", "canopy stops syncing it and signs out. The files in " + as_.a.base + " stay on this computer, and everything stays in Google Drive.", "Remove Account", () => as_.win.api("account_remove", { account: as_.accountId }, () => { as_.close(); as_.win.cn.refresh() }))
            }
        }
    }

    Column {
        visible: as_.tab === 1
        width: parent.width
        spacing: 10
        GLabel { width: parent.width; wrapMode: Text.Wrap; elide: Text.ElideNone; color: Theme.fgSecondary; text: "Google Docs, Sheets and Slides aren't files you can download. Choose how they appear in your folder:" }
        Repeater {
            model: [
                { v: "link", t: "As links (.gddoc, .gdsheet…)", h: "Small files that open the document in your browser. Copying one makes a copy in Google Drive." },
                { v: "ms_office", t: "Converted to Microsoft Office", h: "Word (.docx), Excel (.xlsx), PowerPoint (.pptx). Edit them in LibreOffice; your changes go back into the Google file." },
                { v: "open_document", t: "Converted to OpenDocument", h: "LibreOffice's own formats (.odt, .ods, .odp). Your changes go back into the Google file." }
            ]
            delegate: Item {
                required property var modelData
                width: parent.width
                height: dc.implicitHeight + 8
                GCheck {
                    id: rb
                    checked: (as_.a.export || "link") === modelData.v
                    tip: modelData.h
                    onClicked: {
                        checked = Qt.binding(() => (as_.a.export || "link") === modelData.v)
                        if ((as_.a.export || "link") !== modelData.v)
                            as_.win.confirmSheet("Change how Google files appear?", "canopy replaces them on this computer in the new form (the old copies go to the Trash). Nothing changes in Google Drive.", "Change", () => as_.put({ export: modelData.v }))
                    }
                }
                Column {
                    id: dc
                    anchors.left: rb.right
                    anchors.leftMargin: 4
                    anchors.right: parent.right
                    GLabel { width: parent.width; text: modelData.t }
                    GLabel { width: parent.width; text: modelData.h; font.pixelSize: Theme.small; color: Theme.fgSecondary; wrapMode: Text.Wrap; elide: Text.ElideNone }
                }
            }
        }
    }

    Column {
        visible: as_.tab === 2
        width: parent.width
        spacing: 10
        GLabel { width: parent.width; wrapMode: Text.Wrap; elide: Text.ElideNone; color: Theme.fgSecondary; text: "A conflict is a file changed in two places before canopy could sync it. Whatever you choose, nothing is lost: replaced versions go to a trash." }
        Repeater {
            model: [
                { k: "edit_edit", t: "Changed here and in Google Drive" },
                { k: "local_edit_cloud_delete", t: "Changed here, deleted in Google Drive" },
                { k: "local_delete_cloud_edit", t: "Deleted here, changed in Google Drive" }
            ]
            delegate: Line {
                required property var modelData
                label: modelData.t
                GCombo {
                    width: Math.round(230 * Theme.pt)
                    model: as_.resolutions.map(r => r.t)
                    currentIndex: Math.max(0, as_.resolutions.findIndex(r => r.v === ((as_.a.conflicts || {})[modelData.k] || "keep_both")))
                    tip: "What canopy does in this case"
                    onActivated: i => { const c = Object.assign({}, as_.a.conflicts || {}); c[modelData.k] = as_.resolutions[i].v; as_.put({ conflicts: c }) }
                }
            }
        }
    }

    Column {
        visible: as_.tab === 3
        width: parent.width
        spacing: 8
        GLabel { width: parent.width; wrapMode: Text.Wrap; elide: Text.ElideNone; color: Theme.fgSecondary; text: "One rule per line, like .gitignore: a name (node_modules) matches anywhere; a path (Projects/build) from the folder's top; *.iso every .iso file; Videos/* everything inside Videos; !Videos/keep.mp4 an exception. Case-sensitive. Ignored items stop syncing but aren't deleted." }
        Rectangle {
            width: parent.width
            height: Math.round(180 * Theme.pt)
            radius: Theme.radius
            color: Theme.field
            ScrollView {
                anchors.fill: parent
                anchors.margins: 6
                TextArea {
                    id: rules
                    font.family: Theme.mono
                    font.pixelSize: Theme.body
                    color: Theme.fg
                    renderType: Text.NativeRendering
                    placeholderText: "node_modules\n*.iso\nVideos/Raw"
                    background: null
                }
            }
        }
        Row {
            spacing: 8
            PushButton { primary: true; text: "Apply Rules"; tip: "Save these rules; canopy re-checks every file"; onClicked: { as_.put({ ignore_rules: rules.text.split("\n").map(l => l.trim()).filter(l => l) }); as_.win.toast.show("Ignore rules saved.") } }
            GButton { text: "Help"; flat: false; tip: "More about ignore rules"; onClicked: as_.win.act("help") }
        }
    }

    buttons: [ PushButton { primary: true; text: "Done"; onClicked: as_.close() } ]
}
