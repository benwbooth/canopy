import QtQuick
import QtQuick.Controls.Basic

// The cloud browser (My Drive, Shared with me, Shared drives, Starred) or
// search results: every item with a checkbox for whether it's kept on this
// computer (selective sync), its state, and what you can do with it.
Item {
    id: bv
    property var win
    property bool searching: false
    property string query: ""
    property var items: []
    property bool loading: false
    // Folders opened: [{id, name}]
    property var stack: []
    readonly property string account: win.acct ? win.acct.id : ""
    readonly property string section: win.section
    readonly property string sectionName: ({ my_drive: "My Drive", shared: "Shared with Me", shared_drives: "Shared Drives", starred: "Starred" })[section] || "My Drive"

    onVisibleChanged: if (visible) reload()
    onAccountChanged: { stack = []; if (visible) reload() }
    onSectionChanged: { stack = []; if (visible) reload() }
    onQueryChanged: if (visible && searching) searchDelay.restart()
    Timer { id: searchDelay; interval: 350; onTriggered: bv.reload() }

    function reload() {
        if (searching) {
            if (!query.trim()) { items = []; return }
            loading = true
            win.api("search", { text: query.trim() }, r => { bv.items = r || []; bv.loading = false }, e => { bv.loading = false; win.toast.show(e, [], true) })
            return
        }
        if (!account) return
        loading = true
        const args = { account: account, section: section }
        if (stack.length) args.folder = stack[stack.length - 1].id
        win.api("browse", args, r => { bv.items = r || []; bv.loading = false }, e => { bv.loading = false; win.toast.show(e, [], true) })
    }
    function enter(it) { stack = stack.concat([{ id: it.id, name: it.name }]); reload() }
    function up() { if (stack.length) { stack = stack.slice(0, -1); reload() } }
    function openNamed(n) { const it = items.find(i => i.name === n && i.folder); if (it) enter(it) }
    function activate(it) {
        if (it.folder) enter(it)
        else if (it.local && !it.google) win.cn.openPath(it.local)
        else win.cn.openUrl(it.web)
    }
    function setSynced(it, on) {
        if (section === "shared_drives" && !stack.length) {
            win.api("set_account", { account: account, sync_shared_drives: on }, () => win.toast.show(on ? "Shared drives will sync into “Shared drives” in your Canopy folder." : "Shared drives won't sync any more."))
            return
        }
        if (it.root === null || it.root === undefined) {
            win.toast.show("Items shared with you sync once they're in My Drive: open it on the web and choose Add shortcut to Drive.", [{ text: "Open", run: () => win.cn.openUrl(it.web) }])
            return
        }
        const go = () => win.api("select", { account: account, root: it.root, path: it.rel, synced: on }, () => { bv.reload(); win.toast.show(on ? "“" + it.name + "” will be downloaded." : "“" + it.name + "” is now only in Google Drive; the copy on this computer went to the Trash.") })
        if (!on && (it.sync === "synced" || it.sync === "partial")) {
            win.confirmSheet("Remove “" + it.name + "” from this computer?", "It stays in Google Drive, and you can tick it again any time. The copy on this computer goes to the Trash.", "Remove from This Computer", go)
        } else go()
    }
    function iconFor(it) {
        if (it.folder) return it.shared ? "folder-publicshare|folder" : "folder"
        const m = it.mime || ""
        if (m.indexOf("google-apps.document") >= 0) return "x-office-document"
        if (m.indexOf("google-apps.spreadsheet") >= 0) return "x-office-spreadsheet"
        if (m.indexOf("google-apps.presentation") >= 0) return "x-office-presentation"
        if (m.indexOf("google-apps.drawing") >= 0) return "x-office-drawing"
        if (m.indexOf("google-apps") >= 0) return "text-html"
        return m.replace("/", "-") + "|" + m.split("/")[0] + "-x-generic|application-x-generic|unknown|text-x-generic"
    }
    function stateTip(s) {
        return ({ synced: "On this computer and in Google Drive", pending: "Being downloaded", partial: "Some of what's inside is on this computer", cloud: "Only in Google Drive" })[s] || ""
    }

    // ---- header: path and actions ----
    Item {
        id: head
        width: parent.width
        height: Math.round(52 * Theme.pt)
        Row {
            anchors.left: parent.left
            anchors.leftMargin: 14
            anchors.verticalCenter: parent.verticalCenter
            spacing: 2
            GButton { visible: !bv.searching; enabled: bv.stack.length > 0; iconName: "go-previous|go-up"; tip: Help.tip("Up one folder", "back"); onClicked: bv.up() }
            Repeater {
                model: bv.searching ? [{ id: "", name: "Search: “" + bv.query + "”" }] : [{ id: "", name: bv.sectionName }].concat(bv.stack)
                delegate: Row {
                    required property var modelData
                    required property int index
                    spacing: 2
                    GLabel { visible: index > 0; text: "›"; color: Theme.fgTertiary; anchors.verticalCenter: parent.verticalCenter }
                    GButton {
                        text: modelData.name
                        tip: "Go to " + modelData.name
                        onClicked: { if (!bv.searching) { bv.stack = bv.stack.slice(0, index); bv.reload() } }
                    }
                }
            }
            BusyDots { running: bv.loading; visible: bv.loading; anchors.verticalCenter: parent.verticalCenter }
        }
        Row {
            anchors.right: parent.right
            anchors.rightMargin: 14
            anchors.verticalCenter: parent.verticalCenter
            spacing: 8
            GButton {
                id: newButton
                visible: !bv.searching && bv.section === "my_drive"
                text: "New"
                iconName: "list-add|document-new"
                flat: false
                tip: "Make a new folder, Google Doc, Sheet or Slides here in Google Drive"
                onClicked: newMenu.popup(newButton, 0, newButton.height + 4)
            }
            GButton { visible: !bv.searching; iconName: "view-refresh"; tip: Help.tip("Look again", "syncnow"); onClicked: bv.reload() }
        }
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.separator }
    }
    Menu {
        id: newMenu
        popupType: Popup.Item
        Repeater {
            model: [{ k: "folder", t: "Folder", i: "folder-new" }, { k: "document", t: "Google Doc", i: "x-office-document" }, { k: "spreadsheet", t: "Google Sheet", i: "x-office-spreadsheet" }, { k: "presentation", t: "Google Slides", i: "x-office-presentation" }]
            delegate: MenuItem {
                required property var modelData
                text: modelData.t
                icon.name: modelData.i
                onTriggered: bv.win.askName("New " + modelData.t, modelData.k === "folder" ? "untitled folder" : "Untitled", name => bv.win.api("create", { account: bv.account, parent: bv.stack.length ? bv.stack[bv.stack.length - 1].id : "", name: name, kind: modelData.k }, it => { bv.reload(); if (it.google) bv.win.cn.openUrl(it.web) }))
            }
        }
    }

    // ---- column titles ----
    Row {
        id: cols
        anchors.top: head.bottom
        x: 0
        height: Math.round(26 * Theme.pt)
        width: parent.width
        readonly property int nameW: width - sizeW - dateW - 150
        readonly property int sizeW: 90
        readonly property int dateW: 150
        Item { width: 70; height: 1 }
        GLabel { width: cols.nameW; text: "Name"; font.pixelSize: Theme.small; color: Theme.fgSecondary; anchors.verticalCenter: parent.verticalCenter }
        GLabel { width: cols.sizeW; text: "Size"; font.pixelSize: Theme.small; color: Theme.fgSecondary; horizontalAlignment: Text.AlignRight; anchors.verticalCenter: parent.verticalCenter }
        GLabel { width: cols.dateW; text: "Modified"; font.pixelSize: Theme.small; color: Theme.fgSecondary; leftPadding: 18; anchors.verticalCenter: parent.verticalCenter }
    }

    ListView {
        id: list
        anchors.top: cols.bottom
        anchors.bottom: parent.bottom
        width: parent.width
        clip: true
        model: bv.items
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
        delegate: Rectangle {
            id: row
            required property var modelData
            required property int index
            readonly property var it: modelData
            width: list.width
            height: Theme.rowHeight + 8
            color: rm.containsMouse ? Theme.hover : index % 2 ? Theme.rowAlt : "transparent"
            MouseArea {
                id: rm
                anchors.fill: parent
                hoverEnabled: true
                onDoubleClicked: bv.activate(row.it)
            }
            Row {
                anchors.verticalCenter: parent.verticalCenter
                spacing: 0
                Item {
                    width: 40; height: row.height
                    GCheck {
                        anchors.centerIn: parent
                        tristate: true
                        checkState: row.it.sync === "synced" || row.it.sync === "pending" ? Qt.Checked : row.it.sync === "partial" ? Qt.PartiallyChecked : Qt.Unchecked
                        enabled: row.it.root !== null && row.it.root !== undefined || (bv.section === "shared_drives" && !bv.stack.length)
                        tip: row.it.root !== null && row.it.root !== undefined ? (checkState === Qt.Checked ? "Kept on this computer: untick to keep it only in Google Drive" : "Only in Google Drive: tick to keep a copy on this computer") : "Shared items sync once they're added to My Drive"
                        nextCheckState: function() { return checkState === Qt.Checked ? Qt.Unchecked : Qt.Checked }
                        onClicked: bv.setSynced(row.it, checkState === Qt.Checked)
                    }
                }
                GIcon { name: bv.iconFor(row.it); size: 20; anchors.verticalCenter: parent.verticalCenter }
                Item { width: 10; height: 1 }
                GLabel {
                    width: cols.nameW - (acts.visible ? acts.width : 0)
                    text: row.it.name
                    anchors.verticalCenter: parent.verticalCenter
                }
                Row {
                    id: acts
                    visible: rm.containsMouse || hovered
                    property bool hovered: false
                    spacing: 2
                    anchors.verticalCenter: parent.verticalCenter
                    GButton { visible: !!row.it.local; iconName: "folder-open"; implicitHeight: 24; tip: "Show on this computer"; onClicked: bv.win.cn.showInFiles(row.it.local) }
                    GButton { iconName: "internet-web-browser|applications-internet"; implicitHeight: 24; tip: "Open on drive.google.com"; onClicked: bv.win.cn.openUrl(row.it.web) }
                    GButton { iconName: "edit-copy|insert-link"; implicitHeight: 24; tip: "Copy the link"; onClicked: { bv.win.cn.copyText(row.it.web); bv.win.toast.show("Link copied.") } }
                    GButton { visible: row.it.id && bv.section !== "shared_drives" || bv.stack.length > 0; iconName: "emblem-shared|document-share|send-to"; implicitHeight: 24; tip: "Share…"; onClicked: bv.win.openShare({ account: bv.account, id: row.it.id }) }
                    GButton {
                        visible: !!row.it.id && row.it.starred !== undefined
                        iconName: row.it.starred ? "starred|emblem-favorite" : "non-starred|bookmark-new"
                        implicitHeight: 24
                        tip: row.it.starred ? "Remove the star" : "Star it (find it under Starred)"
                        onClicked: bv.win.api("star", { account: bv.account, id: row.it.id, on: !row.it.starred }, () => bv.reload())
                    }
                }
                GLabel { width: cols.sizeW; text: row.it.folder ? "—" : bv.win.size(row.it.size); color: Theme.fgSecondary; horizontalAlignment: Text.AlignRight; anchors.verticalCenter: parent.verticalCenter }
                Row {
                    width: cols.dateW
                    leftPadding: 18
                    spacing: 6
                    anchors.verticalCenter: parent.verticalCenter
                    GLabel { width: cols.dateW - 48; text: row.it.mtime ? new Date(row.it.mtime * 1000).toLocaleDateString(Qt.locale(), Locale.ShortFormat) : ""; color: Theme.fgSecondary; anchors.verticalCenter: parent.verticalCenter }
                    SyncBadge { sync: row.it.sync === "pending" ? "P" : row.it.sync; size: 14; tip: bv.stateTip(row.it.sync); anchors.verticalCenter: parent.verticalCenter }
                }
            }
        }
        GLabel {
            anchors.centerIn: parent
            visible: !bv.loading && bv.items.length === 0
            text: bv.searching ? (bv.query.trim() ? "Nothing found for “" + bv.query + "”." : "Type in the search field to find files.") : bv.section === "shared_drives" ? "No shared drives." : bv.section === "starred" ? "Nothing starred yet." : "This folder is empty."
            color: Theme.fgSecondary
        }
    }
}
