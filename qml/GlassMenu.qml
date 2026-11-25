import QtQuick
import QtQuick.Controls.Basic

// A small menu in hearth's glass look (as AppMenu's rows): icon, text,
// the accent colour on the row under the pointer, a help line as tooltip.
// Opened from a button, which toggles it:
//     GlassMenu { id: m; anchorItem: btn; model: [{ id, text, icon, tip }]; onTriggered: id => ... }
//     onClicked: m.toggle()
Popup {
    id: gm
    property Item anchorItem
    property var model: []
    property bool alignRight: true
    signal triggered(string id)
    function toggle() { visible ? close() : open() }

    parent: anchorItem
    // Pressing the button itself mustn't close it first (then the click
    // would open it again): the button toggles.
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutsideParent
    popupType: Popup.Item
    x: alignRight && anchorItem ? anchorItem.width - width : 0
    y: anchorItem ? anchorItem.height + 4 : 0
    padding: Theme.menuPad
    topPadding: Theme.menuPad + 0.001
    leftPadding: Theme.menuPad + 0.001
    width: Math.max(Math.round(190 * Theme.pt), rows.implicitWidth + 2 * Theme.menuPad)
    background: GlassPanel { tint: Theme.glassMenu; radius: Theme.glassRadius }
    enter: Transition { NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Theme.fast } }
    exit: Transition { NumberAnimation { property: "opacity"; from: 1; to: 0; duration: Theme.fast } }

    contentItem: Column {
        id: rows
        Repeater {
            model: gm.model
            delegate: Rectangle {
                id: row
                required property var modelData
                width: gm.availableWidth
                implicitWidth: label.implicitWidth + 60
                height: Theme.menuRow
                radius: Theme.radius
                color: hov.hovered ? Theme.accent : "transparent"
                HoverHandler { id: hov }
                GIcon {
                    id: ic
                    x: 8
                    anchors.verticalCenter: parent.verticalCenter
                    name: row.modelData.icon
                    size: 16
                }
                GLabel {
                    id: label
                    anchors.left: ic.right
                    anchors.leftMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.modelData.text
                    color: hov.hovered ? Theme.accentText : Theme.fg
                }
                TapHandler {
                    onTapped: { gm.close(); gm.triggered(row.modelData.id) }
                }
                GTip { visible: hov.hovered && !!row.modelData.tip; text: row.modelData.tip || "" }
            }
        }
    }
}
