//! Moving local files to the freedesktop.org Trash (the home trash, or
//! $topdir/.Trash-$uid on other disks), as grove does, so a file deleted
//! in the cloud can be got back from any file manager's Trash.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

pub fn home_trash() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| crate::paths::home().join(".local/share")).join("Trash")
}

fn mount_top(path: &Path) -> PathBuf {
    let dev = match std::fs::symlink_metadata(path) {
        Ok(m) => m.dev(),
        Err(_) => return PathBuf::from("/"),
    };
    let mut cur = path.to_path_buf();
    while let Some(parent) = cur.parent() {
        match std::fs::metadata(parent) {
            Ok(m) if m.dev() == dev => cur = parent.to_path_buf(),
            _ => break,
        }
    }
    cur
}

/// (trash dir, top the Path= is relative to).
fn trash_for(path: &Path) -> std::io::Result<(PathBuf, Option<PathBuf>)> {
    let home = home_trash();
    let _ = std::fs::create_dir_all(home.join("files"));
    let _ = std::fs::create_dir_all(home.join("info"));
    let home_dev = std::fs::metadata(&home).map(|m| m.dev()).ok();
    if home_dev == Some(std::fs::symlink_metadata(path)?.dev()) {
        return Ok((home, None));
    }
    let top = mount_top(path);
    let shared = top.join(".Trash");
    if let Ok(m) = std::fs::symlink_metadata(&shared) {
        if m.is_dir() && !m.file_type().is_symlink() && m.mode() & 0o1000 != 0 {
            let d = shared.join(uid().to_string());
            if std::fs::create_dir_all(d.join("files")).is_ok() && std::fs::create_dir_all(d.join("info")).is_ok() {
                let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
                return Ok((d, Some(top)));
            }
        }
    }
    let own = top.join(format!(".Trash-{}", uid()));
    std::fs::create_dir_all(own.join("files"))?;
    std::fs::create_dir_all(own.join("info"))?;
    let _ = std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o700));
    Ok((own, Some(top)))
}

fn escape_path(p: &Path) -> String {
    use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
    const SET: &AsciiSet = &CONTROLS.add(b' ').add(b'%').add(b'#').add(b'?').add(b'"').add(b'<').add(b'>').add(b'`').add(b'{').add(b'}');
    utf8_percent_encode(&p.to_string_lossy(), SET).to_string()
}

/// Move `path` to the trash: the real one, or `dir` (tests).
pub fn trash(path: &Path, dir: Option<&Path>) -> Result<PathBuf, String> {
    let (dir, top) = match dir {
        Some(d) => {
            std::fs::create_dir_all(d.join("files")).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(d.join("info")).map_err(|e| e.to_string())?;
            (d.to_path_buf(), None)
        }
        None => trash_for(path).map_err(|e| format!("{}: can't be moved to the Trash ({e})", path.display()))?,
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "item".into());
    let rel = match &top {
        Some(t) => path.strip_prefix(t).map(|p| p.to_path_buf()).unwrap_or_else(|_| path.to_path_buf()),
        None => path.to_path_buf(),
    };
    let info = format!("[Trash Info]\nPath={}\nDeletionDate={}\n", escape_path(&rel), chrono::Local::now().format("%Y-%m-%dT%H:%M:%S"));
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.clone(), String::new()),
    };
    for n in 1..10000 {
        let id = if n == 1 { name.clone() } else { format!("{stem}.{n}{ext}") };
        let info_path = dir.join("info").join(format!("{id}.trashinfo"));
        let mut f = match OpenOptions::new().write(true).create_new(true).open(&info_path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let target = dir.join("files").join(&id);
        if target.symlink_metadata().is_ok() {
            let _ = std::fs::remove_file(&info_path);
            continue;
        }
        let done = f.write_all(info.as_bytes()).and_then(|_| std::fs::rename(path, &target));
        if let Err(e) = done {
            let _ = std::fs::remove_file(&info_path);
            return Err(format!("{}: {e}", path.display()));
        }
        let _ = std::fs::remove_file(dir.join("directorysizes"));
        return Ok(target);
    }
    Err(format!("no free name in the Trash for {name}"))
}
