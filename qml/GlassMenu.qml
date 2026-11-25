import QtQuick
import QtQuick.Controls.Basic

// A small menu in the shared macOS 26 look (MacMenu, from
// hearth/shared/app-menus): icon, text, the accent colour on the row under
// the pointer, a help line as tooltip. Opened from a button, which toggles it:
//     GlassMenu { id: m; anchorItem: btn; model: [{ id, text, icon, tip }]; onTriggered: id => ... }
//     onClicked: m.toggle()
MacMenu {
    id: gm
    property Item anchorItem
    property var model: []
    property bool alignRight: true
    signal triggered(string id)
    function toggle() { visible ? close() : open() }

    dark: Theme.dark
    accent: Theme.accent
    parent: anchorItem
    // Pressing the button itself mustn't close it first (then the click
    // would open it again): the button toggles.
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutsideParent
    x: alignRight && anchorItem ? anchorItem.width - width : 0
    y: anchorItem ? anchorItem.height + 4 : 0

    Instantiator {
        model: gm.model
        delegate: MacMenuItem {
            id: row
            required property var modelData
            text: modelData.text
            icon.name: modelData.icon || ""
            onTriggered: gm.triggered(modelData.id)
            GTip { visible: row.hovered && !!row.modelData.tip; text: row.modelData.tip || "" }
        }
        onObjectAdded: (i, o) => gm.insertItem(i, o)
        onObjectRemoved: (i, o) => gm.removeItem(o)
    }
}
