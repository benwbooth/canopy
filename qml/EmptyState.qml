import QtQuick

// No account yet: what canopy is and the one button to start.
Item {
    id: es
    property var win
    Column {
        anchors.centerIn: parent
        width: Math.min(parent.width - 80, Math.round(460 * Theme.pt))
        spacing: 14
        Image {
            anchors.horizontalCenter: parent.horizontalCenter
            source: Theme.iconUrl("canopy|folder-cloud")
            width: 96; height: 96; sourceSize.width: 96; sourceSize.height: 96
        }
        GLabel { width: parent.width; horizontalAlignment: Text.AlignHCenter; text: "Google Drive on this computer"; font.pixelSize: Theme.large; font.weight: Font.DemiBold }
        GLabel {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            elide: Text.ElideNone
            color: Theme.fgSecondary
            text: "canopy keeps a folder here and your Google Drive the same, both ways, as you work. Choose which folders to keep on this computer, sync any other folder, and open Google Docs from your files."
        }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 10
            PushButton {
                primary: true
                text: "Add Google Account…"
                tip: "Sign in to Google in your browser"
                onClicked: es.win.addAccount(0)
            }
            PushButton {
                text: "Other Clouds…"
                tip: "Dropbox, OneDrive, Box, S3, SFTP, WebDAV and more, set up in rclone (grove's Connect to Server)"
                onClicked: es.win.addAccount(1)
            }
        }
        GLabel {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            elide: Text.ElideNone
            font.pixelSize: Theme.small
            color: Theme.fgTertiary
            text: es.win.status.hearth ? "Your Google accounts from Settings › Internet Accounts are offered there." : es.win.status.has_client ? "" : "First time? canopy needs a Google Cloud sign-in client of your own (five minutes, once). Add Google Account walks you through it."
        }
    }
}
