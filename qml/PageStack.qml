import QtQuick

// The page in the middle of the window, chosen by win.page.
Item {
    id: ps
    property var win
    property string searchText: ""

    function reload() {
        if (activity.visible) activity.reload()
        if (issues.visible) issues.reload()
        if (browser.visible) browser.reload()
        if (search.visible) search.reload()
    }
    function browseUp() { browser.up() }
    function openNewMenu() { browser.openNewMenu() }
    function openFolderNamed(n) { browser.openNamed(n) }
    function firstItemId() { const f = browser.items.find(i => !i.folder) || browser.items[0]; return f ? f.id : "" }

    EmptyState {
        anchors.fill: parent
        visible: ps.win.accounts.length === 0
        win: ps.win
    }
    ActivityView { id: activity; anchors.fill: parent; win: ps.win; visible: ps.win.accounts.length > 0 && ps.win.page === "activity" }
    IssuesView { id: issues; anchors.fill: parent; win: ps.win; conflicts: ps.win.page === "conflicts"; visible: ps.win.accounts.length > 0 && (ps.win.page === "attention" || ps.win.page === "conflicts") }
    BrowserView { id: browser; anchors.fill: parent; win: ps.win; visible: ps.win.accounts.length > 0 && ps.win.page === "browse" }
    BrowserView { id: search; anchors.fill: parent; win: ps.win; searching: true; query: ps.searchText; visible: ps.win.accounts.length > 0 && ps.win.page === "search" }
    FoldersView { anchors.fill: parent; win: ps.win; visible: ps.win.accounts.length > 0 && ps.win.page === "folders" }
}
