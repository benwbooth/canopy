import QtQuick
import QtQuick.Controls.Basic

// Transfers in progress (the toolbar's ↓ button): each file with its bar.
Popup {
    id: pp
    property var win
    property Item anchorItem
    popupType: Popup.Item
    parent: Overlay.overlay
    x: anchorItem ? Math.min(anchorItem.mapToItem(null, 0, 0).x + anchorItem.width - width, (parent ? parent.width : 800) - width - 8) : 0
    y: anchorItem ? anchorItem.mapToItem(null, 0, 0).y + anchorItem.height + 6 : 0
    width: Math.round(380 * Theme.pt)
    padding: 12
    topPadding: 12.001
    leftPadding: 12.001
    readonly property var rows: Object.values(win.transfers)
    background: GlassPanel { tint: Theme.glassMenu; radius: Theme.glassRadius }
    contentItem: Column {
        spacing: 8
        Row {
            width: parent.width
            GLabel { text: pp.rows.length ? "Transfers" : "Nothing is moving"; font.weight: Font.DemiBold; width: parent.width - pauseB.width }
            GButton {
                id: pauseB
                text: pp.win.paused ? "Resume" : "Pause"
                iconName: pp.win.paused ? "media-playback-start" : "media-playback-pause"
                tip: "Pause or resume all syncing"
                onClicked: pp.win.pauseResume()
            }
        }
        GLabel {
            visible: !pp.rows.length
            width: parent.width
            wrapMode: Text.Wrap
            elide: Text.ElideNone
            color: Theme.fgSecondary
            text: pp.win.paused ? "Syncing is paused." : "Uploads and downloads show up here while they run."
        }
        ListView {
            id: kineticList
            KineticWheel { flickable: kineticList; notch: 80 }
            width: parent.width
            height: Math.min(contentHeight, 320)
            clip: true
            spacing: 8
            model: pp.rows
            delegate: Column {
                required property var modelData
                readonly property var t: modelData
                width: ListView.view.width
                spacing: 3
                Row {
                    width: parent.width
                    spacing: 6
                    GLabel { text: t.up ? "↑" : "↓"; color: Theme.accent; font.bold: true }
                    GLabel { width: parent.width - 80; text: t.path.split("/").pop(); }
                    GLabel { width: 60; horizontalAlignment: Text.AlignRight; text: t.total ? Math.round(100 * t.done / t.total) + "%" : pp.win.size(t.done); color: Theme.fgSecondary; font.pixelSize: Theme.small }
                }
                Rectangle {
                    width: parent.width; height: 4; radius: 2; color: Theme.field
                    Rectangle { width: t.total ? parent.width * Math.min(1, t.done / t.total) : 0; height: parent.height; radius: 2; color: Theme.accent }
                }
            }
        }
    }
}
