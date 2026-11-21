use std::path::{Path, PathBuf};

pub fn register_bundled() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let theme = gtk::IconTheme::for_display(&display);
    for dir in bundled_icon_dirs() {
        theme.add_search_path(dir);
    }
}

fn bundled_icon_dirs() -> Vec<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin_dir) = exe.parent()
    {
        let installed = bin_dir.join("../share/shrimply/icons");
        if installed.is_dir() {
            return vec![installed];
        }
    }
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let mut dirs = vec![repo_root.join("assets/icons")];
    let adwaita = repo_root.join("external/adwaita-icon-theme/Adwaita/symbolic");
    if let Ok(entries) = std::fs::read_dir(&adwaita) {
        let mut subdirs = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        subdirs.sort();
        dirs.extend(subdirs);
    }
    dirs
}
