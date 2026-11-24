import QtQuick

// "Are you sure?" or "What name?": ask(title, message, okText, run) /
// askName(title, default, run).
Sheet {
    id: cs
    property string okText: "OK"
    property bool destructive: false
    property bool withName: false
    property var run: null
    function ask(t, m, ok, f) { title = t; message = m; okText = ok; withName = false; destructive = /remove|delete|stop/i.test(ok); run = f; open() }
    function askName(t, def, f) { title = t; message = ""; okText = "Create"; withName = true; destructive = false; run = f; field.text = def; open(); field.forceActiveFocus(); field.selectAll() }
    GField {
        id: field
        visible: cs.withName
        width: parent.width
        onAccepted: go.clicked()
    }
    buttons: [
        PushButton { text: "Cancel"; onClicked: cs.close() },
        PushButton {
            id: go
            primary: !cs.destructive
            destructive: cs.destructive
            text: cs.okText
            enabled: !cs.withName || field.text.trim() !== ""
            onClicked: { cs.close(); if (cs.run) cs.run(cs.withName ? field.text.trim() : undefined) }
        }
    ]
}
