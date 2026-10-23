//! Freedesktop launchers and PNG/SVG icons from the user's icon theme.
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Component, Path, PathBuf},
};

type Groups = HashMap<String, HashMap<String, String>>;

fn groups(text: &str) -> Groups {
    let mut result = Groups::new();
    let mut section = String::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|name| name.strip_suffix(']'))
        {
            section = name.to_owned();
        } else if let Some((key, value)) = line.split_once('=') {
            result
                .entry(section.clone())
                .or_default()
                .insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    result
}

#[derive(Clone, Debug)]
pub(super) struct DesktopEntry {
    pub name: Option<String>,
    icon: Option<String>,
    executable: Option<String>,
    application: bool,
    dbus: bool,
    command_present: bool,
}

impl DesktopEntry {
    pub(super) fn launchable(&self) -> bool {
        self.application && (self.dbus || self.command_present)
    }
}

pub(super) fn desktop_entry(path: &Path) -> Option<DesktopEntry> {
    let text = fs::read_to_string(path).ok()?;
    parse_desktop_entry(&text)
}

fn parse_desktop_entry(text: &str) -> Option<DesktopEntry> {
    let sections = groups(text);
    let section = sections.get("Desktop Entry")?;
    Some(DesktopEntry {
        name: section.get("Name").cloned(),
        icon: section.get("Icon").cloned(),
        executable: section
            .get("TryExec")
            .filter(|value| !value.is_empty())
            .or_else(|| section.get("Exec"))
            .cloned(),
        application: section
            .get("Type")
            .is_some_and(|value| value == "Application"),
        dbus: section
            .get("DBusActivatable")
            .is_some_and(|value| value == "true"),
        command_present: section
            .get("Exec")
            .is_some_and(|value| !value.trim().is_empty()),
    })
}

fn data_dirs() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        roots.push(PathBuf::from(home));
    } else if let Some(home) = env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".local/share"));
    }
    if let Some(dirs) = env::var_os("XDG_DATA_DIRS").filter(|value| !value.is_empty()) {
        roots.extend(env::split_paths(&dirs));
    } else {
        roots.extend([
            PathBuf::from("/usr/local/share"),
            PathBuf::from("/usr/share"),
        ]);
    }
    roots.retain(|path| path.is_absolute());
    roots
}

// Only used to associate an executable with its icon, never to execute a command.
fn executable_word(command: &str) -> String {
    let mut quoted = false;
    let mut escaped = false;
    let mut word = String::new();
    for ch in command.trim_start().chars() {
        if escaped {
            word.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        } else if ch.is_whitespace() && !quoted {
            break;
        } else {
            word.push(ch);
        }
    }
    word
}

fn entry_in_dirs(path: &Path, roots: &[PathBuf]) -> Option<DesktopEntry> {
    if super::extension_is(path, "desktop") {
        return desktop_entry(path);
    }
    let name = path.file_name()?;
    let mut candidates = Vec::new();
    for root in roots {
        let applications = root.join("applications");
        let direct = applications.join(format!("{}.desktop", name.to_string_lossy()));
        if let Some(entry) = desktop_entry(&direct).filter(DesktopEntry::launchable) {
            return Some(entry);
        }
        if let Ok(entries) = fs::read_dir(applications) {
            candidates.extend(
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|entry| super::extension_is(entry, "desktop")),
            );
        }
    }
    candidates.sort();
    for candidate in candidates {
        let Some(entry) = desktop_entry(&candidate).filter(DesktopEntry::launchable) else {
            continue;
        };
        let Some(command) = &entry.executable else {
            continue;
        };
        let word = executable_word(command);
        if Path::new(&word).file_name() == Some(name) {
            return Some(entry);
        }
    }
    None
}

pub(super) fn entry_for_app(path: &Path) -> Option<DesktopEntry> {
    entry_in_dirs(path, &data_dirs())
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn theme_icon(
    roots: &[PathBuf],
    theme: &str,
    icon: &str,
    visited: &mut HashSet<String>,
) -> Option<PathBuf> {
    if !safe_relative(theme) || !visited.insert(theme.to_owned()) || visited.len() > 32 {
        return None;
    }
    let mut inherited = Vec::new();
    let mut candidates = Vec::new();
    for root in roots {
        let base = root.join(theme);
        let sections = fs::read_to_string(base.join("index.theme"))
            .ok()
            .map(|text| groups(&text))
            .unwrap_or_default();
        if let Some(index) = sections.get("Icon Theme") {
            if let Some(inherits) = index.get("Inherits") {
                inherited.extend(inherits.split(',').map(str::trim).map(str::to_owned));
            }
            for dir in index
                .get("Directories")
                .into_iter()
                .chain(index.get("ScaledDirectories"))
                .flat_map(|dirs| dirs.split(','))
                .map(str::trim)
                .filter(|dir| safe_relative(dir))
            {
                let section = sections.get(dir);
                let number = |key: &str, default: i32| {
                    section
                        .and_then(|s| s.get(key))
                        .and_then(|value| value.parse::<i32>().ok())
                        .unwrap_or(default)
                };
                let size = number("Size", 64).clamp(1, 8192);
                let scale = number("Scale", 1).clamp(1, 16);
                let score = (size * scale - 64).abs();
                for (rank, extension) in ["png", "svg"].into_iter().enumerate() {
                    let candidate = base.join(dir).join(format!("{icon}.{extension}"));
                    if candidate.is_file() {
                        candidates.push((score, rank, candidate));
                    }
                }
            }
        }
        // Minimal themes and vendor installs sometimes omit index.theme.
        for dir in [
            "64x64/apps",
            "48x48/apps",
            "scalable/apps",
            "32x32/apps",
            "256x256/apps",
            "",
        ] {
            for extension in ["png", "svg"] {
                let candidate = base.join(dir).join(format!("{icon}.{extension}"));
                if candidate.is_file() {
                    candidates.push((64, 2, candidate));
                }
            }
        }
    }
    candidates.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
    if let Some((_, _, path)) = candidates.into_iter().next() {
        return Some(path);
    }
    for parent in inherited {
        if let Some(path) = theme_icon(roots, &parent, icon, visited) {
            return Some(path);
        }
    }
    None
}

fn resolve_icon(
    icon: &str,
    data: &[PathBuf],
    legacy_icons: Option<PathBuf>,
    theme: &str,
) -> Option<PathBuf> {
    let path = Path::new(icon);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_owned());
    }
    if !safe_relative(icon) || path.components().count() != 1 {
        return None;
    }
    let icon = if path
        .extension()
        .is_some_and(|extension| extension == "png" || extension == "svg")
    {
        path.file_stem()?.to_str()?
    } else {
        icon
    };
    let mut roots: Vec<PathBuf> = data.iter().map(|root| root.join("icons")).collect();
    if let Some(legacy) = legacy_icons {
        roots.insert(0, legacy);
    }
    let mut visited = HashSet::new();
    if let Some(path) = theme_icon(&roots, theme, icon, &mut visited)
        .or_else(|| theme_icon(&roots, "hicolor", icon, &mut visited))
    {
        return Some(path);
    }
    for root in data {
        for extension in ["png", "svg"] {
            let candidate = root.join("pixmaps").join(format!("{icon}.{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub(super) fn icon_for_app(path: &Path) -> Option<PathBuf> {
    let data = data_dirs();
    let icon = entry_in_dirs(path, &data)?.icon?;
    let theme = crate::core::process::background_command("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|theme| theme.trim().trim_matches('\'').to_owned())
        .unwrap_or_else(|| "hicolor".to_owned());
    let legacy = env::var_os("HOME").map(|home| PathBuf::from(home).join(".icons"));
    resolve_icon(&icon, &data, legacy, &theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_application_shortcut_reads_only_its_main_section() {
        let entry = parse_desktop_entry("[Desktop Entry]\nType=Application\nName=Blender\nExec=\"/opt/My Blender/blender\" %f\nIcon=blender\n[Desktop Action Other]\nName=Wrong\nIcon=wrong\n").unwrap();
        assert!(entry.launchable());
        assert_eq!(entry.name.as_deref(), Some("Blender"));
        assert_eq!(entry.icon.as_deref(), Some("blender"));
        assert_eq!(
            executable_word(entry.executable.as_deref().unwrap()),
            "/opt/My Blender/blender"
        );
        assert!(
            !parse_desktop_entry("[Desktop Entry]\nType=Link\nURL=https://example.com\n")
                .unwrap()
                .launchable()
        );
    }

    #[test]
    fn desktop_application_shortcut_resolves_inherited_theme_icons() {
        let root = super::super::tests::scratch();
        fs::create_dir_all(root.join("applications")).unwrap();
        fs::create_dir_all(root.join("icons/hicolor/64x64/apps")).unwrap();
        fs::create_dir_all(root.join("icons/custom")).unwrap();
        fs::write(
            root.join("applications/org.blender.Blender.desktop"),
            "[Desktop Entry]\nType=Application\nName=Blender\nExec=blender %f\nIcon=blender\n",
        )
        .unwrap();
        fs::write(
            root.join("icons/custom/index.theme"),
            "[Icon Theme]\nInherits=hicolor\n",
        )
        .unwrap();
        let icon = root.join("icons/hicolor/64x64/apps/blender.svg");
        fs::write(&icon, "<svg/>").unwrap();
        let entry = entry_in_dirs(Path::new("/usr/bin/blender"), &[root.clone()]).unwrap();
        assert_eq!(entry.name.as_deref(), Some("Blender"));
        assert_eq!(
            resolve_icon(
                entry.icon.as_deref().unwrap(),
                &[root.clone()],
                None,
                "custom"
            ),
            Some(icon)
        );
        assert!(resolve_icon("../escape", &[root.clone()], None, "custom").is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_application_shortcut_launches_through_gio_without_shell_parsing() {
        let root = super::super::tests::scratch();
        let path = root.join("My App.desktop");
        fs::write(
            &path,
            "[Desktop Entry]\nType=Application\nName=App\nExec=app %U\n",
        )
        .unwrap();
        let command = super::super::launch_command(&path).unwrap();
        assert_eq!(command.get_program(), "gio");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [std::ffi::OsStr::new("launch"), path.as_os_str()]
        );
        fs::remove_dir_all(root).unwrap();
    }
}
