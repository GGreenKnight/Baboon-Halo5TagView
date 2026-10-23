//! Native application shortcuts. Paths are passed as arguments, never shell commands.
#[cfg(any(unix, test))]
use std::fs;
#[cfg(any(not(windows), test))]
use std::path::PathBuf;
use std::{path::Path, process::Command};

#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[path = "app_shortcut/linux.rs"]
mod linux;

#[cfg(not(windows))]
pub(crate) enum AppIcon {
    File(PathBuf),
    #[cfg(target_os = "macos")]
    Png(Vec<u8>),
}

fn extension_is(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

pub(crate) fn validate_app_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if extension_is(path, "app") && path.is_dir() {
        let executable =
            macos_bundle_executable(path).ok_or("Select a valid application bundle.")?;
        return validate_executable(&executable);
    }
    if !path.is_file() {
        return Err("Select an existing application or executable.".to_owned());
    }
    #[cfg(target_os = "linux")]
    if extension_is(path, "desktop") {
        return linux::desktop_entry(path)
            .filter(linux::DesktopEntry::launchable)
            .map(|_| ())
            .ok_or_else(|| "Select a desktop launcher with an application command.".to_owned());
    }
    #[cfg(windows)]
    if !extension_is(path, "exe") {
        return Err("Select an .exe application.".to_owned());
    }
    // Windows kit tools can be launched through Wine without Unix execute bits.
    if extension_is(path, "exe") {
        return Ok(());
    }
    validate_executable(path)
}

fn validate_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(path)
            .map_err(|error| format!("Could not inspect application: {error}"))?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err("This file is not executable. Choose an executable application or enable its execute permission.".to_owned());
        }
    }
    let _ = path;
    Ok(())
}

/// Use the native launch service for bundles/desktop entries, and direct spawn for binaries.
pub(crate) fn launch_command(path: &Path) -> Result<Command, String> {
    validate_app_path(path)?;
    let path = std::path::absolute(path)
        .map_err(|error| format!("Could not resolve application path: {error}"))?;
    #[cfg(target_os = "macos")]
    if extension_is(&path, "app") && path.is_dir() {
        let mut command = Command::new("/usr/bin/open");
        command.arg("-a").arg(&path).arg("--args");
        return Ok(command);
    }
    #[cfg(target_os = "linux")]
    if extension_is(&path, "desktop") {
        let mut command = Command::new("gio");
        command.arg("launch").arg(path);
        return Ok(command);
    }
    #[cfg(unix)]
    if extension_is(&path, "exe") {
        let mut command = Command::new("wine");
        command.arg(path);
        return Ok(command);
    }
    Ok(Command::new(path))
}

pub(crate) fn display_name(path: &Path) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSBundle, NSString};
        if let Some(bundle) = NSBundle::bundleWithPath(&NSString::from_str(path.to_str()?)) {
            for key in ["CFBundleDisplayName", "CFBundleName"] {
                if let Some(value) = bundle.objectForInfoDictionaryKey(&NSString::from_str(key)) {
                    if let Ok(name) = value.downcast::<NSString>() {
                        if !name.is_empty() {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(entry) = linux::entry_for_app(path) {
        return entry.name;
    }
    let _ = path;
    None
}

#[cfg(not(windows))]
pub(crate) fn native_icon(path: &Path) -> Option<AppIcon> {
    if !path.exists() {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
        use objc2_foundation::{NSDictionary, NSString};
        let native = objc2::rc::autoreleasepool(|_| {
            let icon =
                NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path.to_str()?));
            let tiff = icon.TIFFRepresentation()?;
            let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)?;
            // SAFETY: The dictionary is empty, so every property has a valid type.
            let png = unsafe {
                bitmap.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
            }?;
            Some(AppIcon::Png(png.to_vec()))
        });
        if native.is_some() {
            return native;
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(icon) = linux::icon_for_app(path) {
        return Some(AppIcon::File(icon));
    }
    // Portable sidecars also support unpacked applications and custom tools.
    for extension in ["png", "svg", "ico"] {
        let candidate = path.with_extension(extension);
        if candidate.is_file() {
            return Some(AppIcon::File(candidate));
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn macos_bundle_executable(path: &Path) -> Option<PathBuf> {
    use objc2_foundation::{NSBundle, NSString};
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(path.to_str()?))?;
    Some(PathBuf::from(bundle.executablePath()?.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn scratch() -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("baboon-native-shortcut-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn application_shortcut_command_keeps_spaces_and_shell_characters_literal() {
        let root = scratch();
        let executable = root.join(if cfg!(windows) {
            "my app & tool.exe"
        } else {
            "my app & tool"
        });
        fs::write(&executable, []).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let command = launch_command(&executable).unwrap();
        assert_eq!(Path::new(command.get_program()), executable);
        assert_eq!(command.get_args().count(), 0);
        assert!(validate_app_path(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn native_shortcut_rejects_non_executable_files_and_uses_wine_for_windows_tools() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch();
        let executable = root.join("blender");
        fs::write(&executable, []).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(validate_app_path(&executable).is_err());
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(validate_app_path(&executable).is_ok());
        let windows_tool = root.join("halo3_tag_test.exe");
        fs::write(&windows_tool, []).unwrap();
        let command = launch_command(&windows_tool).unwrap();
        assert_eq!(command.get_program(), "wine");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [windows_tool.as_os_str()]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_application_shortcut_supports_bundles_and_native_icons() {
        let root = scratch();
        let bundle = root.join("Test Application.app");
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::write(bundle.join("Contents/Info.plist"), r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>test-app</string><key>CFBundleName</key><string>Test Application</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleIdentifier</key><string>org.baboon.shortcut.test</string></dict></plist>"#).unwrap();
        let executable = bundle.join("Contents/MacOS/test-app");
        fs::copy("/usr/bin/true", &executable).unwrap();
        assert!(validate_app_path(&bundle).is_ok());
        assert_eq!(display_name(&bundle).as_deref(), Some("Test Application"));
        let command = launch_command(&bundle).unwrap();
        assert_eq!(command.get_program(), "/usr/bin/open");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                std::ffi::OsStr::new("-a"),
                bundle.as_os_str(),
                std::ffi::OsStr::new("--args")
            ]
        );
        assert!(
            matches!(native_icon(&bundle), Some(AppIcon::Png(bytes)) if bytes.starts_with(b"\x89PNG"))
        );
        fs::remove_dir_all(root).unwrap();
    }
}
