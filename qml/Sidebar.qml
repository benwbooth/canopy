import QtQuick
import QtQuick.Controls.Basic

// Finder-style sidebar: canopy's feeds, then each account with its
// sections, then adding an account or a folder.
Rectangle {
    id: sb
    property var win
    color: Theme.sidebar

    component Row_: Item {
        id: r
        property string label: ""
        property string icon: ""
        property bool selected: false
        property int count: 0
        property bool alert: false
        property string tip: ""
        property int indent: 0
        signal clicked()
        width: ListView.view ? ListView.view.width : parent.width
        height: Theme.sidebarRow
        Rectangle {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            radius: Theme.radius
            color: r.selected ? Theme.accent : ma.containsMouse ? Theme.hover : "transparent"
        }
        Row {
            anchors.left: parent.left
            anchors.leftMargin: 16 + r.indent
            anchors.verticalCenter: parent.verticalCenter
            spacing: 8
            GIcon { name: r.icon; size: 16; anchors.verticalCenter: parent.verticalCenter; tint: r.selected ? Theme.accentText : "transparent" }
            GLabel { text: r.label; color: r.selected ? Theme.accentText : Theme.fg; width: Math.min(implicitWidth, sb.width - 90 - r.indent) }
        }
        Rectangle {
            visible: r.count > 0
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            height: 17; width: Math.max(17, n.implicitWidth + 10); radius: 8.5
            color: r.selected ? Qt.rgba(1, 1, 1, 0.25) : r.alert ? Theme.danger : Theme.field
            GLabel { id: n; anchors.centerIn: parent; text: String(r.count); font.pixelSize: Theme.caption; font.bold: true; color: r.alert || r.selected ? "white" : Theme.fgSecondary }
        }
        MouseArea { id: ma; anchors.fill: parent; hoverEnabled: true; onClicked: r.clicked() }
        GTip { visible: ma.containsMouse && r.tip !== ""; text: r.tip }
    }
    component Header_: Item {
        property string label: ""
        property string sync: ""
        property string tip: ""
        property bool menu: false
        signal settings()
        width: parent.width
        height: Math.round(30 * Theme.pt)
        GLabel {
            anchors.left: parent.left
            anchors.leftMargin: 16
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 4
            text: parent.label
            font.pixelSize: Theme.small
            font.weight: Font.DemiBold
            color: Theme.fgSecondary
            width: Math.min(implicitWidth, sb.width - 70)
        }
        SyncBadge { anchors.right: gear.left; anchors.rightMargin: 4; anchors.bottom: parent.bottom; anchors.bottomMargin: 5; sync: parent.sync; size: 13; tip: parent.tip }
        GButton {
            id: gear
            visible: parent.menu
            anchors.right: parent.right
            anchors.rightMargin: 8
            anchors.bottom: parent.bottom
            implicitHeight: 22
            iconName: "configure|preferences-system"
            iconSize: 13
            tip: "This account's settings: Google Docs, conflicts, ignore rules, folder, sign out"
            onClicked: parent.settings()
        }
    }

    Flickable {
        id: sideFlick
        anchors.fill: parent
        anchors.topMargin: 6
        contentHeight: col.implicitHeight + 20
        clip: true
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
        KineticWheel { flickable: sideFlick; notch: Theme.sidebarRow * 2 }
        Column {
            id: col
            width: parent.width
            Header_ { label: "Canopy" }
            Row_ { width: col.width; label: "Activity"; icon: "document-open-recent|view-history|appointment-soon"; selected: sb.win.page === "activity"; tip: Help.tip("What canopy uploaded, downloaded, moved and deleted", "activity"); onClicked: sb.win.go("activity") }
            Row_ { width: col.width; label: "Needs Attention"; icon: "dialog-warning"; count: sb.win.errorCount; alert: true; selected: sb.win.page === "attention"; tip: Help.tip("Files canopy couldn't sync, and why", "attention"); onClicked: sb.win.go("attention") }
            Row_ { width: col.width; label: "Conflicts"; icon: "dialog-question|vcs-conflicting"; count: sb.win.conflictCount; alert: true; selected: sb.win.page === "conflicts"; tip: Help.tip("Files changed in two places: choose which version to keep", "conflicts"); onClicked: sb.win.go("conflicts") }

            Repeater {
                model: sb.win.accountIds
                delegate: Column {
                    required property var modelData
                    // Looked up by id: updates in place while syncing.
                    readonly property var a: sb.win.accounts.find(x => x.id === modelData) || ({ id: modelData, email: "", roots: [], state: {} })
                    width: col.width
                    readonly property string phase: (a.state || {}).phase || ""
                    // Synced through rclone (Dropbox, OneDrive, S3...): no
                    // Google sections.
                    readonly property bool google: a.kind !== "rclone"
                    Header_ {
                        label: google ? a.email : (a.name || a.email)
                        menu: true
                        sync: a.paused ? "paused" : phase === "idle" ? "synced" : phase
                        tip: phase === "signed_out" ? "Sign in again to keep syncing" : phase === "offline" ? "Offline" : a.paused ? "Paused" : phase === "syncing" ? "Syncing…" : "Up to date"
                        onSettings: accountSheetOpen(a.id)
                    }
                    Row_ {
                        visible: phase === "signed_out"
                        width: col.width; label: "Sign In Again…"; icon: "dialog-password|system-lock-screen"; alert: true
                        tip: "Google access for this account stopped working. Sign in to continue syncing."
                        onClicked: sb.win.signInAgain(a.id)
                    }
                    Row_ { width: col.width; label: google ? "My Drive" : "All Files"; icon: "folder-cloud|folder-remote"; selected: sb.win.page === "browse" && sb.win.account === a.id && sb.win.section === "my_drive"; tip: google ? "Everything in your Drive: tick what to keep on this computer" : "Everything in " + (a.name || "this cloud") + " (" + a.email + "): tick what to keep on this computer"; onClicked: sb.win.go("browse", a.id, "my_drive") }
                    Row_ { visible: google; width: col.width; label: "Shared with Me"; icon: "folder-publicshare|folder-remote"; selected: sb.win.page === "browse" && sb.win.account === a.id && sb.win.section === "shared"; tip: "Files other people shared with you"; onClicked: sb.win.go("browse", a.id, "shared") }
                    Row_ { visible: google; width: col.width; label: "Shared Drives"; icon: "network-server|folder-remote"; selected: sb.win.page === "browse" && sb.win.account === a.id && sb.win.section === "shared_drives"; tip: "Your team's shared drives"; onClicked: sb.win.go("browse", a.id, "shared_drives") }
                    Row_ { visible: google; width: col.width; label: "Starred"; icon: "starred|emblem-favorite"; selected: sb.win.page === "browse" && sb.win.account === a.id && sb.win.section === "starred"; tip: "Files you starred in Google Drive"; onClicked: sb.win.go("browse", a.id, "starred") }
                    Row_ {
                        width: col.width; label: "Synced Folders"; icon: "folder-sync|folder"
                        count: (a.roots || []).length
                        selected: sb.win.page === "folders" && sb.win.account === a.id
                        tip: Help.tip("The folders on this computer kept in sync with this account", "folders")
                        onClicked: sb.win.go("folders", a.id)
                    }
                }
            }
            Item { width: 1; height: 10 }
            Row_ { width: col.width; label: "Add Account…"; icon: "list-add-user|list-add"; tip: "Sync another Google account, or Dropbox, OneDrive, S3 and other clouds set up in rclone (each gets its own folder)"; onClicked: sb.win.act("addaccount") }
            Row_ { width: col.width; visible: sb.win.accounts.length > 0; label: "Sync a Folder…"; icon: "folder-new|list-add"; tip: Help.tip("Keep any folder on this computer in sync with Google Drive (or back it up)", "addfolder"); onClicked: sb.win.act("addfolder") }
        }
    }
    function accountSheetOpen(id) { sb.win.account = id; sb.win.act("accountsettings") }
}
