// Qt-side helpers canopy's Rust code calls: the application (native text),
// the QML engine with its theme-icon provider, and the screenshot hook.
#pragma once

#include <QtCore/QString>
#include "rust/cxx.h"

// The application object: native text rendering, names, fonts, icons.
void canopy_create_app();
// Create the QML engine, load `url`, run the event loop.
int canopy_run(const QString& url, const QString& importPath);
// The icon theme to match light or dark.
void canopy_set_icon_theme(bool dark);
// Test hook: save every visible window (the first to `path`, others to
// path-1.png, path-2.png...).
void canopy_grab_windows(const QString& path);
// Put text on the clipboard (GUI thread).
void canopy_copy_text(const QString& text);
