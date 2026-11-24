//! Ignore rules: Insync's built-in names plus the account's own
//! gitignore-style lines (bare names at any depth, `dir/path`, `*.ext`,
//! `dir/*`, `!exceptions`), matched case-sensitively relative to each root.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

/// canopy's own working folder inside each root (partial downloads).
pub const PART_DIR: &str = ".canopy-part";

pub struct Ignore {
    user: Option<Gitignore>,
}

/// Names no one wants synced (editor locks, temp files), as Insync.
pub fn builtin(name: &str, is_dir: bool) -> bool {
    const STARTS: [&str; 6] = [".~lock", "~$", ".~$", "._", "#", ".trash-"];
    const ENDS: [&str; 8] = [".tmp", ".swp", "~", "___jb_tmp___", "___jb_old___", ".asta.bak", ".idlk", ".insyncdl"];
    if name == PART_DIR {
        return true;
    }
    if is_dir && (name.starts_with(".Trash-") || name == ".Trash") {
        return true;
    }
    STARTS.iter().any(|s| name.starts_with(s)) || ENDS.iter().any(|s| name.ends_with(s)) || name.contains("(autosaved)")
}

impl Ignore {
    pub fn new(rules: &[String]) -> Self {
        let lines: Vec<&String> = rules.iter().filter(|l| !l.trim().is_empty()).collect();
        if lines.is_empty() {
            return Ignore { user: None };
        }
        // The root is virtual: rules are matched against relative paths.
        let mut b = GitignoreBuilder::new("/");
        b.case_insensitive(false).ok();
        for l in lines {
            if let Err(e) = b.add_line(None, l) {
                eprintln!("canopy: ignore rule {l:?} skipped: {e}");
            }
        }
        Ignore { user: b.build().ok() }
    }

    /// `rel` is '/'-separated, relative to the root.
    pub fn ignored(&self, rel: &str, is_dir: bool) -> bool {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        if builtin(name, is_dir) {
            return true;
        }
        match &self.user {
            Some(g) => g.matched_path_or_any_parents(Path::new("/").join(rel), is_dir).is_ignore(),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        let ig = Ignore::new(&["Images".into(), "*.iso".into(), "Work/build/*".into(), "!Work/build/keep.txt".into(), "Projects/target".into()]);
        assert!(ig.ignored("Images", true));
        assert!(ig.ignored("a/b/Images", true));
        assert!(ig.ignored("a/Images/x.png", false));
        assert!(!ig.ignored("images", true));
        assert!(ig.ignored("d/disk.iso", false));
        assert!(!ig.ignored("d/disk.ISO", false));
        assert!(ig.ignored("Work/build/out.o", false));
        assert!(!ig.ignored("Work/build/keep.txt", false));
        assert!(ig.ignored("Projects/target/debug/x", false));
        assert!(!ig.ignored("Other/Projects/target", true));
        assert!(ig.ignored("doc/.~lock.report.odt#", false));
        assert!(ig.ignored("a/file.swp", false));
        assert!(ig.ignored(".canopy-part", true));
        assert!(ig.ignored(".Trash-1000", true));
        assert!(!ig.ignored("notes.txt", false));
    }
}
