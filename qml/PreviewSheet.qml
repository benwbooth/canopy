import QtQuick
import QtQuick.Controls.Basic

// What syncing would do now, without doing it (canopy's dry run): every
// upload, download, move and deletion, per synced folder.
Sheet {
    id: pv
    property var win
    property string account: ""
    property string root: ""
    property var result: ({})
    property bool loading: false
    readonly property var roots: Object.keys(result).map(k => Object.assign({ id: k }, result[k]))
    title: "What Syncing Would Do"
    message: "Nothing has been changed yet."
    width: Math.min(parent ? parent.width - 40 : 700, Math.round(640 * Theme.pt))
    function openFor(acc, r) {
        account = acc; root = r || ""; result = {}; loading = true
        open()
        win.api("preview", { account: acc, root: root }, res => { pv.result = res || {}; pv.loading = false }, e => { pv.loading = false; pv.win.toast.show(e, [], true) })
    }
    BusyDots { running: pv.loading; visible: pv.loading }
    ListView {
        width: parent.width
        height: Math.min(contentHeight, (pv.parent ? pv.parent.height : 700) - 260)
        clip: true
        model: pv.roots
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
        delegate: Column {
            required property var modelData
            width: ListView.view.width - 12
            spacing: 4
            GLabel { text: modelData.name + "  (" + (modelData.plan.length ? modelData.plan.length + " change" + (modelData.plan.length === 1 ? "" : "s") : "nothing to do") + ")"; font.weight: Font.DemiBold }
            GLabel { visible: !!modelData.held; width: parent.width; text: modelData.held || ""; color: Theme.danger; wrapMode: Text.Wrap; elide: Text.ElideNone }
            Repeater {
                model: modelData.plan.slice(0, 500)
                delegate: GLabel {
                    required property string modelData
                    width: parent.width
                    text: modelData
                    font.family: Theme.mono
                    font.pixelSize: Theme.small
                    color: /trash|Trash|stop syncing/.test(modelData) ? Theme.danger : Theme.fgSecondary
                    elide: Text.ElideMiddle
                }
            }
            GLabel { visible: modelData.plan.length > 500; text: "…and " + (modelData.plan.length - 500) + " more"; color: Theme.fgTertiary }
            Item { width: 1; height: 8 }
        }
    }
    buttons: [ PushButton { primary: true; text: "Done"; onClicked: pv.close() } ]
}
