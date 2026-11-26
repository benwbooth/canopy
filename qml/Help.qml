pragma Singleton
import QtQuick

// Every action with its shortcut (for menus, tooltips and the Shortcuts
// page) and the help topics (the searchable Help window).
QtObject {
    readonly property var shortcuts: [
        { id: "activity", group: "Go", action: "Activity (what canopy did)", keys: ["Ctrl+1"] },
        { id: "attention", group: "Go", action: "Needs attention (errors)", keys: ["Ctrl+2"] },
        { id: "conflicts", group: "Go", action: "Conflicts", keys: ["Ctrl+3"] },
        { id: "mydrive", group: "Go", action: "My Drive (choose what to sync)", keys: ["Ctrl+4"] },
        { id: "folders", group: "Go", action: "Synced folders", keys: ["Ctrl+5"] },
        { id: "search", group: "Go", action: "Search your clouds", keys: ["Ctrl+F"] },
        { id: "back", group: "Go", action: "Up one folder (in My Drive)", keys: ["Alt+Up", "Backspace"] },

        { id: "pause", group: "Sync", action: "Pause or resume syncing", keys: ["Ctrl+P"] },
        { id: "syncnow", group: "Sync", action: "Look for changes now", keys: ["F5", "Ctrl+R"] },
        { id: "progress", group: "Sync", action: "Show transfers in progress", keys: ["Ctrl+T"] },
        { id: "addfolder", group: "Sync", action: "Sync a folder from this computer…", keys: ["Ctrl+Shift+N"] },
        { id: "openbase", group: "Sync", action: "Open the Google Drive folder", keys: ["Ctrl+O"] },

        { id: "addaccount", group: "canopy", action: "Add a Google account…", keys: [] },
        { id: "accountsettings", group: "canopy", action: "Account settings (Google Docs, conflicts, ignore rules)", keys: ["Ctrl+Shift+,"] },
        { id: "settings", group: "canopy", action: "Settings", keys: ["Ctrl+,"] },
        { id: "sidebar", group: "canopy", action: "Show or hide the sidebar", keys: ["Ctrl+Alt+S"] },
        { id: "close", group: "canopy", action: "Close the window (syncing goes on)", keys: ["Ctrl+W"] },
        { id: "quit", group: "canopy", action: "Quit canopy (stops syncing)", keys: ["Ctrl+Q"] },

        { id: "help", group: "Help", action: "Canopy Help", keys: ["F1"] },
        { id: "shortcutlist", group: "Help", action: "Keyboard shortcuts", keys: ["Ctrl+/"] }
    ]

    function entry(id) {
        for (const s of shortcuts) if (s.id === id) return s
        return null
    }
    function key(id) {
        const e = entry(id)
        return e && e.keys.length ? e.keys[0] : ""
    }
    function tip(text, id) {
        const k = key(id)
        return k ? text + "  (" + k + ")" : text
    }

    readonly property var topics: [
        { title: "What canopy does", body: "canopy keeps a folder on this computer and your Google Drive the same. Change, add or delete a file on either side and canopy does the same on the other side within seconds.\nYour files are in ~/Canopy/<your email>/My Drive (you can move it). canopy keeps syncing while its window is closed: the cloud icon in the menu bar shows what it's doing." },
        { title: "Setting up Google sign-in (once)", body: "On hearth, Google accounts come from Settings › Internet Accounts (set up once for every app): Add Google Account in canopy lists them. If an account says it needs permission for all your Drive files, click Sign In Again and allow “See, edit, create and delete all of your Google Drive files”.\nWithout hearth, canopy signs in by itself. Google only lets apps it has checked use your whole Drive, so canopy signs in through a Google Cloud \"client\" of your own. It takes about five minutes, once:\n1. Go to console.cloud.google.com and create a project (any name, e.g. \"canopy\").\n2. APIs & Services › Library: find \"Google Drive API\" and click Enable.\n3. APIs & Services › OAuth consent screen (Google Auth Platform): choose External, fill in an app name and your email. Under Audience add your Google account as a test user, then click Publish app (In production) so your sign-in doesn't expire every 7 days. Google will warn that the app isn't verified: that's expected for your own client.\n4. Credentials › Create credentials › OAuth client ID › Application type \"Desktop app\". Copy the Client ID and Client secret.\n5. In canopy: Settings › Google sign-in, paste both, then Add Account.\nWhen you sign in, Google shows \"Google hasn't verified this app\": click Advanced › Go to (your app name). Tick every permission it asks for." },
        { title: "Choosing what to sync (selective sync)", body: "Open My Drive in the sidebar. Every file and folder has a checkbox: ticked items are kept on this computer, unticked ones stay only in Google Drive (you can still see them here and open them on the web).\nUntick a folder you no longer need on this computer: canopy moves its local copy to the Trash and keeps it in Google Drive. Tick it again to bring it back.\nA half-filled box means some things inside are synced and some aren't." },
        { title: "Syncing any folder (backup)", body: "Click + Sync a Folder (or File › Sync a Folder…) and pick a folder anywhere on this computer, for example Pictures or a project folder. canopy makes a folder of the same name in your Google Drive and keeps them in sync.\nChoose how:\n• Two-way: changes go both ways (the usual).\n• Back up to Google Drive: only this computer's changes go up; the cloud copy follows this folder (a backup).\n• Download only: the folder follows Google Drive; changes made here aren't uploaded.\nIn grove you can right-click a folder › Sync with Google Drive." },
        { title: "Google Docs, Sheets and Slides", body: "Google's own files have no file to download, so canopy shows them one of two ways (Account settings › Google Docs):\n• As links (the usual): a small .gddoc / .gdsheet / .gdslides file that opens the document in your browser when you double-click it. Copying a link file in the folder makes a copy of the document in Google Drive.\n• Converted to Microsoft Office (.docx, .xlsx, .pptx) or OpenDocument (.odt, .ods, .odp): you can edit them in LibreOffice; when you save, canopy puts your changes back into the Google file, which stays a Google file.\nForms, scripts and maps are always links." },
        { title: "Conflicts", body: "A conflict is a file changed in two places before canopy could sync it (for example on two computers while one was offline), or changed in one place and deleted in the other.\nBy default canopy keeps both: your version is renamed \"name (conflict computer date)\" and uploaded too, so nothing is lost. Account settings › Conflicts lets you choose instead to always keep this computer's version, Google Drive's, or to ask each time (they're then listed under Conflicts)." },
        { title: "Deleted files", body: "Nothing canopy deletes is gone for good:\n• A file deleted in Google Drive moves to this computer's Trash (open it from grove's sidebar).\n• A file you delete here moves to Google Drive's trash (drive.google.com › Trash) for 30 days.\nIf the whole folder goes missing (an unplugged disk, a renamed folder), canopy stops and asks instead of deleting anything in the cloud." },
        { title: "Ignore rules", body: "Account settings › Ignore Rules lists files canopy leaves alone, one rule per line (the same rules as .gitignore):\n• A name, like node_modules, matches files and folders with that name anywhere.\n• A path from the top of the folder, like Projects/build, matches just that.\n• *.iso matches every file ending in .iso; Videos/* everything inside Videos.\n• Start a line with ! to make an exception: !Videos/keep.mp4\nRules are case-sensitive. Editor lock and temporary files (~$doc, .~lock, *.swp, *.tmp) are always ignored. Things that become ignored stop syncing but aren't deleted anywhere." },
        { title: "Sharing", body: "Right-click a file in grove › Google Drive › Share…, or use Share in My Drive. You can copy the link, let anyone with the link view (or comment, or edit), and invite people by email with a role. Removing someone takes their access away." },
        { title: "Badges in grove", body: "grove shows each file's sync state on its icon: ✓ synced, ↻ syncing, ✕ couldn't sync (open canopy › Needs Attention to see why), ! a conflict, – ignored. Right-click a synced file for Google Drive actions: copy link, view on the web, share, star, remove from this computer." },
        { title: "Pausing", body: "Click Pause in the toolbar (or the menu bar icon › Pause syncing) to stop syncing everything, for example on a slow connection. Changes made meanwhile are synced when you resume. Each account can also be paused on its own in its settings." },
        { title: "Limits and the network", body: "Settings › Network: limit upload and download speeds, how many files transfer at once, and a proxy if your network needs one. canopy retries by itself when Google Drive is busy or the network drops; big uploads and downloads continue where they stopped." },
        { title: "Something isn't syncing", body: "• Look at Needs Attention: every file canopy couldn't sync is there with the reason, and Retry.\n• Names Google Drive can't take, or files you have no permission to change, show up there too.\n• Symbolic links aren't synced (Google Drive can't hold them): sync the folder they point to instead.\n• Check the account isn't paused and that you're signed in (the sidebar shows a warning if not)." },
        { title: "The command line", body: "canopy works without the window too. In a terminal:\n  canopy status            what's happening\n  canopy status -f FILE    one file's state\n  canopy pause, canopy resume    stop and start syncing\n  canopy account add    sign in (--paste without a browser)\n  canopy selective-sync    see and change what's synced\n  canopy error list    what couldn\'t sync\n  canopy conflict list    conflicts waiting for you\n  canopy --help            everything\ncanopyd (the part that syncs) can run as a systemd user service on a computer with no screen." },
        { title: "Dropbox, OneDrive and other clouds", body: "Besides Google Drive, canopy syncs a folder with any cloud rclone knows: Dropbox, OneDrive, Box, pCloud, Mega, Amazon S3 and other storage buckets, SFTP and WebDAV servers (Nextcloud…), and many more. The files are really on this computer (nothing is mounted), and canopy keeps them in step both ways, with the same care as for Google Drive.\nIn canopy click Add Account… › Other Clouds. The first time, Add a Cloud Drive opens Settings › Accounts › Cloud Drives: pick the service and sign in (once, for all your apps). Then click Sync next to it. Its folder is ~/Canopy/<the cloud's name>. To sync only part of it (or one bucket), type the folder in \"Only a folder of it\".\nWhat's different from Google Drive:\n• Changes made elsewhere are found by looking at the whole cloud about once a minute (Look for Changes Now looks at once).\n• Deleted files go to the cloud's own trash where it has one (Dropbox, OneDrive, Box, pCloud, Mega); elsewhere canopy moves them into a .canopy-trash folder at the top of the cloud, which you can empty when you like.\n• Clouds where names differ only in capital letters are the same name (Dropbox, OneDrive) can't hold both \"Notes.txt\" and \"notes.txt\": canopy leaves one here and lists it under Needs Attention.\n• Google Docs, sharing, stars and shared drives are Google's own and aren't there.\nFrom the command line: canopy account remotes, then canopy account add --rclone NAME:" },
        { title: "Coming from Insync", body: "canopy uses the same link files as Insync (.gddoc, .gdsheet…). You can point canopy at your Insync folder: stop Insync, add your account in canopy, then in Account settings move the folder to where Insync kept it (e.g. ~/Insync/you@gmail.com/Google Drive). canopy matches the files that are already there by content instead of downloading them again." }
    ]
}
