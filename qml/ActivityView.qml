import QtQuick
import QtQuick.Controls.Basic

// Activity: everything canopy did, newest first, by day, filterable.
Item {
    id: av
    property var win
    property var rows: []
    property int filter: 0
    readonly property var filters: [
        { text: "All", kinds: [] , tip: "Everything" },
        { text: "Uploads", kinds: ["upload", "edit"], tip: "Files sent to the cloud" },
        { text: "Downloads", kinds: ["download", "add"], tip: "Files brought to this computer" },
        { text: "Moves", kinds: ["move", "rename"], tip: "Renamed or moved files" },
        { text: "Deletions", kinds: ["delete", "local_delete", "unsync"], tip: "Files moved to a trash or removed from this computer" },
        { text: "Other", kinds: ["conflict", "ignore", "share"], tip: "Conflicts, sharing and ignored files" }
    ]
    readonly property var shown: filter === 0 ? rows : rows.filter(r => filters[filter].kinds.indexOf(r.kind) >= 0)
    ListModel { id: lm }
    property int shownFilter: -1
    function row(r) { return { key: r.account + "/" + r.id, at: r.at, kind: r.kind, path: r.path, detail: r.detail || "", account: r.account, root: r.root, dayText: day(r.at) } }
    // New entries go in at the top; the rows already shown stay (no
    // flicker, the scroll position holds).
    onShownChanged: {
        const keys = new Set()
        for (let i = 0; i < lm.count; i++) keys.add(lm.get(i).key)
        const incremental = shownFilter === filter && lm.count > 0 && shown.length >= lm.count && keys.has(shown[shown.length - 1] ? row(shown[Math.min(shown.length - 1, lm.count - 1)]).key : "")
        if (!incremental) {
            lm.clear()
            for (const r of shown) lm.append(row(r))
        } else {
            let at = 0
            for (const r of shown) {
                const k = row(r).key
                if (keys.has(k)) break
                lm.insert(at++, row(r))
            }
            while (lm.count > shown.length) lm.remove(lm.count - 1)
        }
        shownFilter = filter
    }

    onVisibleChanged: if (visible) reload()
    function reload() { win.api("activity", { limit: 400 }, r => { if (!win.same(av.rows, r || [])) av.rows = r || [] }) }

    function rootPath(r) {
        const a = win.accounts.find(x => x.id === r.account)
        const root = a ? (a.roots || []).find(x => x.id === r.root) : null
        return root ? root.local + "/" + r.path : ""
    }
    function kindIcon(k) {
        return ({ upload: "cloud-upload|go-up", edit: "document-edit|go-up", download: "cloud-download|download|go-down", add: "list-add|folder-new",
                  move: "go-next|document-send", rename: "edit-rename|document-edit", delete: "user-trash|edit-delete", local_delete: "user-trash|edit-delete",
                  unsync: "list-remove|edit-delete", conflict: "dialog-warning", share: "emblem-shared|document-share", ignore: "dialog-cancel|action-unavailable" })[k] || "dialog-information"
    }
    // Where a top-level item is: its folder's name ("My Drive", "Dropbox"),
    // with the account when there are several.
    function topName(r) {
        const a = win.accounts.find(x => x.id === r.account)
        const root = a ? (a.roots || []).find(x => x.id === r.root) : null
        const name = root ? root.name : "My Drive"
        if (win.accounts.length < 2 || !a) return name
        return a.kind === "rclone" ? name : name + " (" + a.email + ")"
    }
    function kindText(k, r) {
        return ({ upload: "Uploaded", edit: "Uploaded changes", download: "Downloaded", add: "Added", move: "Moved", rename: "Renamed",
                  delete: "Deleted in " + win.cloudName(r ? r.account : ""), local_delete: "Moved to the Trash here", unsync: "Removed from this computer",
                  conflict: "Conflict", share: "Sharing changed", ignore: "Ignored" })[k] || k
    }
    function day(t) {
        const d = new Date(t * 1000), now = new Date()
        const y = new Date(now.getTime() - 86400000)
        if (d.toDateString() === now.toDateString()) return "Today"
        if (d.toDateString() === y.toDateString()) return "Yesterday"
        return d.toLocaleDateString(Qt.locale(), "dddd d MMMM yyyy")
    }

    Item {
        id: head
        width: parent.width
        height: Math.round(52 * Theme.pt)
        GLabel { anchors.left: parent.left; anchors.leftMargin: 20; anchors.verticalCenter: parent.verticalCenter; text: "Activity"; font.pixelSize: Theme.title; font.weight: Font.DemiBold }
        Segmented {
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            model: av.filters
            current: av.filter
            onPicked: i => av.filter = i
        }
    }
    ListView {
        id: list
        anchors.top: head.bottom
        anchors.bottom: parent.bottom
        width: parent.width
        clip: true
        model: lm
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
        KineticWheel { flickable: list; notch: Math.round(44 * Theme.pt) * 2 }
        section.property: "dayText"
        section.delegate: Item {
            required property string section
            width: list.width
            height: Math.round(30 * Theme.pt)
            GLabel { anchors.left: parent.left; anchors.leftMargin: 20; anchors.bottom: parent.bottom; anchors.bottomMargin: 4; text: parent.section; font.pixelSize: Theme.small; font.weight: Font.DemiBold; color: Theme.fgSecondary }
        }
        delegate: Item {
            id: row
            required property var model
            readonly property var r: model
            width: list.width
            height: Math.round(44 * Theme.pt)
            HoverHandler { id: rowHover }
            Rectangle { anchors.fill: parent; anchors.leftMargin: 10; anchors.rightMargin: 10; radius: Theme.radius; color: rowHover.hovered ? Theme.hover : "transparent" }
            GIcon { id: ic; x: 22; anchors.verticalCenter: parent.verticalCenter; name: av.kindIcon(row.r.kind); size: 20 }
            Column {
                anchors.left: ic.right
                anchors.leftMargin: 12
                anchors.right: when.left
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                GLabel { width: parent.width; text: row.r.path.split("/").pop() || row.r.path }
                GLabel { width: parent.width; font.pixelSize: Theme.small; color: Theme.fgSecondary; text: av.kindText(row.r.kind, row.r) + (row.r.detail ? " · " + row.r.detail : "") + " · " + (row.r.path.indexOf("/") >= 0 ? row.r.path.substring(0, row.r.path.lastIndexOf("/")) : av.topName(row.r)) }
            }
            GLabel { id: when; anchors.right: show.left; anchors.rightMargin: 8; anchors.verticalCenter: parent.verticalCenter; text: new Date(row.r.at * 1000).toLocaleTimeString(Qt.locale(), "HH:mm"); color: Theme.fgSecondary; font.pixelSize: Theme.small }
            GButton {
                id: show
                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                opacity: rowHover.hovered ? 1 : 0
                Behavior on opacity { NumberAnimation { duration: Theme.fast } }
                iconName: "folder-open"
                tip: "Show in its folder"
                onClicked: { const p = av.rootPath(row.r); if (p) av.win.cn.showInFiles(p) }
            }
        }
        GLabel {
            anchors.centerIn: parent
            visible: av.shown.length === 0
            text: av.rows.length ? "Nothing of this kind yet." : "Nothing has happened yet. Changes show up here as canopy syncs them."
            color: Theme.fgSecondary
        }
    }
}
