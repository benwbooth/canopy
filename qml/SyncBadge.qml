import QtQuick

// A round sync-state mark: synced ✓ (green), syncing ↻ (accent, turning),
// paused ‖, offline, error ! (red), conflict, cloud-only ☁. Used in the
// toolbar, the sidebar and file lists (grove draws its own from the same
// states).
Item {
    id: b
    property string sync: ""      // overall states, or S/P/E/C/I, or synced/pending/partial/cloud
    property int size: 16
    property string tip: ""
    width: size
    height: size
    readonly property string kind: ({
        "synced": "ok", "S": "ok", "idle": "ok",
        "syncing": "busy", "P": "busy", "pending": "busy", "starting": "busy", "connecting": "busy",
        "paused": "paused",
        "offline": "offline",
        "error": "error", "E": "error", "signed_out": "error",
        "C": "conflict",
        "I": "ignored",
        "partial": "partial",
        "cloud": "cloud",
        "empty": "cloud"
    })[sync] || ""
    visible: kind !== ""

    Rectangle {
        anchors.fill: parent
        radius: width / 2
        color: b.kind === "ok" ? Theme.good : b.kind === "busy" ? Theme.accent : b.kind === "error" || b.kind === "conflict" ? Theme.danger
             : b.kind === "partial" ? Qt.rgba(Theme.good.r, Theme.good.g, Theme.good.b, 0.55) : b.kind === "cloud" ? "transparent" : Theme.fgTertiary
        border.width: b.kind === "cloud" ? 1.2 : 0
        border.color: Theme.fgSecondary
    }
    GLabel {
        id: mark
        x: (parent.width - width) / 2
        y: (parent.height - height) / 2
        text: ({ "ok": "✓", "busy": "↻", "paused": "‖", "offline": "–", "error": "!", "conflict": "!", "ignored": "–", "partial": "✓", "cloud": "☁" })[b.kind] || ""
        color: b.kind === "cloud" ? Theme.fgSecondary : "white"
        font.pixelSize: Math.round(b.size * 0.62)
        font.bold: true
        RotationAnimation on rotation {
            running: b.kind === "busy" && !Theme.reduceMotion
            loops: Animation.Infinite
            from: 0; to: 360; duration: 1600
        }
        onTextChanged: rotation = 0
    }
    MouseArea { id: ma; anchors.fill: parent; hoverEnabled: b.tip !== ""; acceptedButtons: Qt.NoButton }
    GTip { visible: ma.containsMouse && b.tip !== ""; text: b.tip }
}
