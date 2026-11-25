import QtQuick
import QtQuick.Controls.Basic

// Canopy Help: topics in a list on the left, the chosen one laid out on the
// right (paragraphs, numbered steps, bullets, commands in a box), and every
// keyboard shortcut by group with key caps. One search field filters both.
Sheet {
    id: hw
    property int page: 0          // 0 topics, 1 shortcuts
    property int current: 0       // index into `topics`
    title: "Canopy Help"
    width: Math.min(parent ? parent.width - 40 : 820, Math.round(760 * Theme.pt))

    function openAt(p) { page = p; search.text = ""; current = 0; open(); search.forceActiveFocus() }

    readonly property string q: search.text.trim().toLowerCase()
    readonly property var topics: Help.topics.filter(t => q === "" || (t.title + " " + t.body).toLowerCase().indexOf(q) >= 0)
    readonly property var keys: Help.shortcuts.filter(s => q === "" || (s.action + " " + s.keys.join(" ") + " " + s.group).toLowerCase().indexOf(q) >= 0)
    // The shortcuts by group, in their order: [{ group, rows }].
    readonly property var keyGroups: {
        const out = []
        for (const s of keys) {
            if (!out.length || out[out.length - 1].group !== s.group) out.push({ group: s.group, rows: [] })
            out[out.length - 1].rows.push(s)
        }
        return out
    }
    readonly property var topic: topics.length ? topics[Math.min(current, topics.length - 1)] : null
    onQChanged: current = 0

    // A topic's body as blocks: paragraphs, lists (lines starting "• " or
    // "1. ") and command boxes (lines indented two spaces: the command, then
    // after two or more spaces what it does).
    function blocks(body) {
        const out = []
        const last = () => out.length ? out[out.length - 1] : null
        for (const line of body.split("\n")) {
            let m
            if (line.startsWith("  ")) {
                const p = line.trim().split(/\s{2,}/)
                if (!last() || last().kind !== "code") out.push({ kind: "code", rows: [] })
                last().rows.push({ cmd: p[0], desc: p.slice(1).join(" ") })
            } else if ((m = line.match(/^•\s*(.*)$/)) || (m = line.match(/^(\d+\.)\s+(.*)$/))) {
                const item = m.length > 2 ? { mark: m[1], text: m[2] } : { mark: "•", text: m[1] }
                if (!last() || last().kind !== "list") out.push({ kind: "list", items: [] })
                last().items.push(item)
            } else if (line.trim() !== "") {
                out.push({ kind: "para", text: line.trim() })
            }
        }
        return out
    }

    Item {
        width: parent.width
        height: Math.round(30 * Theme.pt)
        Segmented {
            id: seg
            anchors.verticalCenter: parent.verticalCenter
            model: [{ text: "Help", tip: "How canopy syncs and how to set it up" }, { text: "Keyboard Shortcuts", tip: "Every shortcut" }]
            current: hw.page
            onPicked: i => hw.page = i
        }
        GField {
            id: search
            anchors.left: seg.right
            anchors.leftMargin: 10
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            iconName: "edit-find|system-search"
            placeholderText: hw.page === 0 ? "Search help" : "Search shortcuts"
            clearable: true
            tip: "Find help topics and shortcuts"
            Keys.onDownPressed: if (hw.page === 0) hw.current = Math.min(hw.current + 1, hw.topics.length - 1)
            Keys.onUpPressed: if (hw.page === 0) hw.current = Math.max(hw.current - 1, 0)
        }
    }

    // ---- the pages ----
    Item {
        id: pages
        width: parent.width
        height: Math.max(Math.round(300 * Theme.pt), Math.min(Math.round(520 * Theme.pt), (hw.parent ? hw.parent.height : 700) - 200))

        // Help: topic list | the topic.
        Item {
            anchors.fill: parent
            visible: hw.page === 0
            Rectangle {
                id: listPane
                width: Math.round(210 * Theme.pt)
                height: parent.height
                radius: Theme.glassRadius
                color: Theme.rowAlt
                ListView {
                    id: topicList
                    KineticWheel { flickable: topicList; notch: 120 }
                    anchors.fill: parent
                    anchors.margins: 6
                    clip: true
                    spacing: 1
                    model: hw.topics
                    currentIndex: hw.current
                    ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
                    delegate: AbstractButton {
                        id: tb
                        required property int index
                        required property var modelData
                        width: ListView.view.width
                        height: Math.max(Theme.sidebarRow, tl.implicitHeight + 10)
                        hoverEnabled: true
                        onClicked: hw.current = index
                        background: Rectangle {
                            radius: Theme.radius
                            color: tb.index === hw.current ? Theme.accent : tb.hovered ? Theme.hover : "transparent"
                        }
                        contentItem: GLabel {
                            id: tl
                            leftPadding: 8
                            rightPadding: 8
                            text: tb.modelData.title
                            wrapMode: Text.Wrap
                            elide: Text.ElideNone
                            color: tb.index === hw.current ? Theme.accentText : Theme.fg
                        }
                    }
                }
                GLabel {
                    anchors.centerIn: parent
                    width: parent.width - 24
                    visible: hw.topics.length === 0
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    elide: Text.ElideNone
                    text: "Nothing matches “" + search.text + "”."
                    color: Theme.fgSecondary
                }
            }

            Flickable {
                id: topicView
                KineticWheel { flickable: topicView; notch: 120 }
                anchors.left: listPane.right
                anchors.leftMargin: 22
                anchors.right: parent.right
                height: parent.height
                clip: true
                contentHeight: topicCol.implicitHeight + 8
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
                Connections { target: hw; function onTopicChanged() { topicView.contentY = 0 } }

                Column {
                    id: topicCol
                    width: topicView.width - 14
                    spacing: 12
                    GLabel {
                        width: parent.width
                        text: hw.topic ? hw.topic.title : ""
                        font.pixelSize: Theme.large
                        font.weight: Font.Bold
                        wrapMode: Text.Wrap
                        elide: Text.ElideNone
                        bottomPadding: 2
                    }
                    Repeater {
                        model: hw.topic ? hw.blocks(hw.topic.body) : []
                        delegate: Loader {
                            required property var modelData
                            width: topicCol.width
                            property var b: modelData
                            sourceComponent: modelData.kind === "list" ? listC : modelData.kind === "code" ? codeC : paraC
                        }
                    }
                }
            }
        }

        // Shortcuts: by group, key caps on the right.
        Flickable {
            id: keyView
            KineticWheel { flickable: keyView; notch: 120 }
            anchors.fill: parent
            visible: hw.page === 1
            clip: true
            contentHeight: keyCol.implicitHeight + 8
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
            Column {
                id: keyCol
                width: keyView.width - 14
                spacing: 18
                Repeater {
                    model: hw.keyGroups
                    delegate: Column {
                        id: grp
                        required property var modelData
                        width: keyCol.width
                        spacing: 4
                        GLabel {
                            text: grp.modelData.group.charAt(0).toUpperCase() + grp.modelData.group.slice(1)
                            font.weight: Font.DemiBold
                            color: Theme.fgSecondary
                            leftPadding: 10
                            bottomPadding: 2
                        }
                        Rectangle {
                            width: parent.width
                            height: rows.implicitHeight
                            radius: Theme.glassRadius
                            color: Theme.rowAlt
                            Column {
                                id: rows
                                width: parent.width
                                Repeater {
                                    model: grp.modelData.rows
                                    delegate: Item {
                                        id: kr
                                        required property int index
                                        required property var modelData
                                        width: rows.width
                                        height: Math.max(Math.round(34 * Theme.pt), act.implicitHeight + 12)
                                        Rectangle {
                                            visible: kr.index > 0
                                            x: 12; width: parent.width - 24; height: 1
                                            color: Theme.separator
                                        }
                                        GLabel {
                                            id: act
                                            x: 14
                                            width: parent.width - caps.width - 40
                                            anchors.verticalCenter: parent.verticalCenter
                                            text: kr.modelData.action
                                            wrapMode: Text.Wrap
                                            elide: Text.ElideNone
                                        }
                                        Row {
                                            id: caps
                                            anchors.right: parent.right
                                            anchors.rightMargin: 14
                                            anchors.verticalCenter: parent.verticalCenter
                                            spacing: 6
                                            GLabel {
                                                visible: kr.modelData.keys.length === 0
                                                text: "No shortcut"
                                                color: Theme.fgTertiary
                                                font.pixelSize: Theme.small
                                            }
                                            Repeater {
                                                model: kr.modelData.keys
                                                delegate: Row {
                                                    required property int index
                                                    required property string modelData
                                                    spacing: 6
                                                    GLabel {
                                                        visible: index > 0
                                                        text: "or"
                                                        color: Theme.fgTertiary
                                                        font.pixelSize: Theme.small
                                                        anchors.verticalCenter: parent.verticalCenter
                                                    }
                                                    Repeater {
                                                        // Each key of the chord its own cap.
                                                        model: modelData === "+" ? ["+"] : modelData.split(/\+(?!$)/)
                                                        delegate: Rectangle {
                                                            required property string modelData
                                                            width: Math.max(height, kl.implicitWidth + 12)
                                                            height: Math.round(20 * Theme.pt)
                                                            radius: Math.round(5 * Theme.pt)
                                                            color: Theme.field
                                                            border.width: 1
                                                            border.color: Theme.separator
                                                            anchors.verticalCenter: parent.verticalCenter
                                                            GLabel {
                                                                id: kl
                                                                anchors.centerIn: parent
                                                                text: parent.modelData
                                                                font.pixelSize: Theme.small
                                                                font.weight: Font.Medium
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            GLabel {
                anchors.centerIn: parent
                visible: hw.keys.length === 0
                text: "Nothing matches “" + search.text + "”."
                color: Theme.fgSecondary
            }
        }
    }

    // ---- topic blocks ----
    Component {
        id: paraC
        GLabel {
            text: parent.b.text
            wrapMode: Text.Wrap
            elide: Text.ElideNone
            lineHeight: 1.12
        }
    }
    Component {
        id: listC
        Column {
            readonly property var b: parent.b
            spacing: 7
            Repeater {
                model: parent.b.items
                delegate: Item {
                    id: li
                    required property var modelData
                    width: parent.width
                    height: lt.implicitHeight
                    GLabel {
                        width: Math.round(22 * Theme.pt)
                        horizontalAlignment: Text.AlignRight
                        verticalAlignment: Text.AlignTop
                        text: li.modelData.mark
                        color: Theme.fgSecondary
                        font.features: { "tnum": 1 }
                    }
                    GLabel {
                        id: lt
                        x: Math.round(30 * Theme.pt)
                        width: parent.width - x
                        verticalAlignment: Text.AlignTop
                        text: li.modelData.text
                        wrapMode: Text.Wrap
                        elide: Text.ElideNone
                        lineHeight: 1.12
                    }
                }
            }
        }
    }
    Component {
        id: codeC
        Rectangle {
            readonly property var b: parent.b
            height: grid.implicitHeight + 20
            radius: Theme.radius + 2
            color: Theme.field
            Grid {
                id: grid
                x: 14; y: 10
                columns: 2
                columnSpacing: 22
                rowSpacing: 5
                Repeater {
                    model: grid.parent.b.rows.length * 2
                    delegate: GLabel {
                        required property int index
                        readonly property var r: grid.parent.b.rows[Math.floor(index / 2)]
                        text: index % 2 === 0 ? r.cmd : r.desc
                        font.family: index % 2 === 0 ? Theme.mono : Theme.family
                        font.pixelSize: index % 2 === 0 ? Theme.small + 0.5 : Theme.body
                        color: index % 2 === 0 ? Theme.fg : Theme.fgSecondary
                        // (Never zero wide: Grid skips empty cells, shifting the rest.)
                        width: Math.max(1, implicitWidth)
                        height: Math.round(18 * Theme.pt)
                    }
                }
            }
        }
    }

    buttons: [
        PushButton { primary: true; text: "Done"; onClicked: hw.close() }
    ]
}
