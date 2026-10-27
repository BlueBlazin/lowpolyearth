//! Runtime storage locations for interactive native launches.
//!
//! `VEARTH_RUNTIME_ROOT` overrides all locations. Otherwise, unpackaged
//! launches store data relative to the current working directory. A
//! Finder-launched `.app` has no useful working-directory contract, so the
//! packaged application uses macOS directories, including its sandbox container
//! when App Sandbox is enabled.

use std::path::{Path, PathBuf};

pub const PRODUCT_NAME: &str = "Low Poly Earth";
pub const BUNDLE_IDENTIFIER: &str = "com.blueblazin.voxelearth";
pub const WINDOW_TITLE: &str = PRODUCT_NAME;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePaths {
    pub cache: PathBuf,
    pub application_support: PathBuf,
    pub photos: PathBuf,
    pub logs: PathBuf,
    pub packaged: bool,
}

impl RuntimePaths {
    /// Browser storage is the origin-private file system, whose paths are
    /// relative to its root; photos are offered as downloads instead.
    #[cfg(target_arch = "wasm32")]
    pub fn discover() -> Self {
        Self::under_root(Path::new(""), true)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn discover() -> Self {
        if let Some(root) = std::env::var_os("VEARTH_RUNTIME_ROOT") {
            return Self::under_root(Path::new(&root), false);
        }

        let packaged = std::env::current_exe()
            .ok()
            .as_deref()
            .is_some_and(executable_is_in_app_bundle);
        if packaged {
            let mut paths = Self::packaged_for_home(home_directory());
            if let Some(pictures) = pictures_directory() {
                paths.photos = pictures.join(PRODUCT_NAME);
            }
            return paths;
        }

        let root = std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir());
        Self::under_root(&root, false)
    }

    pub fn ensure_directories(&self) -> std::io::Result<()> {
        for directory in [
            &self.cache,
            &self.application_support,
            &self.photos,
            &self.logs,
        ] {
            vearth_platform::fs::create_dir_all(directory)?;
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn packaged_for_home(home: PathBuf) -> Self {
        let library = home.join("Library");
        Self {
            cache: library.join("Caches").join(BUNDLE_IDENTIFIER),
            application_support: library.join("Application Support").join(BUNDLE_IDENTIFIER),
            photos: home.join("Pictures").join(PRODUCT_NAME),
            logs: library.join("Logs").join(PRODUCT_NAME),
            packaged: true,
        }
    }

    fn under_root(root: &Path, packaged: bool) -> Self {
        Self {
            cache: root.join("cache"),
            application_support: root.join("output").join("application-support"),
            photos: root.join("captures").join("photos"),
            logs: root.join("output").join("logs"),
            packaged,
        }
    }
}

#[cfg(target_os = "macos")]
fn home_directory() -> PathBuf {
    objc2::rc::autoreleasepool(|_| {
        // Foundation returns the app container for sandboxed processes. HOME
        // is an inherited environment variable and is not the authority here.
        let home = unsafe { objc2_foundation::NSHomeDirectory().as_ref().to_string() };
        PathBuf::from(home)
    })
}

#[cfg(target_os = "macos")]
fn pictures_directory() -> Option<PathBuf> {
    use objc2_foundation::{
        NSSearchPathDirectory, NSSearchPathDomainMask, NSSearchPathForDirectoriesInDomains,
    };
    objc2::rc::autoreleasepool(|_| {
        // Use the system's user-directory mapping; the signed app requests
        // Pictures access for its automatic PNG and recipe exports.
        let directories = unsafe {
            NSSearchPathForDirectoriesInDomains(
                NSSearchPathDirectory::NSPicturesDirectory,
                NSSearchPathDomainMask::NSUserDomainMask,
                objc2::runtime::Bool::YES,
            )
            .as_ref()
        };
        directories
            .first()
            .map(|path| PathBuf::from(path.to_string()))
    })
}

#[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
fn pictures_directory() -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn desktop_directory() -> std::io::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let desktop = objc2::rc::autoreleasepool(|_| {
        use objc2_foundation::{
            NSSearchPathDirectory, NSSearchPathDomainMask, NSSearchPathForDirectoriesInDomains,
        };
        let directories = unsafe {
            NSSearchPathForDirectoriesInDomains(
                NSSearchPathDirectory::NSDesktopDirectory,
                NSSearchPathDomainMask::NSUserDomainMask,
                objc2::runtime::Bool::YES,
            )
            .as_ref()
        };
        directories
            .first()
            .map(|path| PathBuf::from(path.to_string()))
    });
    #[cfg(not(target_os = "macos"))]
    let desktop = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|home| home.join("Desktop"));

    desktop.ok_or_else(|| std::io::Error::other("Could not locate your Desktop folder"))
}

#[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(not(target_arch = "wasm32"))]
fn executable_is_in_app_bundle(executable: &Path) -> bool {
    executable.ancestors().any(|ancestor| {
        ancestor
            .extension()
            .is_some_and(|extension| extension == "app")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_only_paths_nested_in_an_app_bundle() {
        assert!(executable_is_in_app_bundle(Path::new(
            "/Applications/Low Poly Earth.app/Contents/MacOS/Low Poly Earth"
        )));
        assert!(!executable_is_in_app_bundle(Path::new(
            "/work/target/release/vearth-app"
        )));
    }

    #[test]
    fn packaged_paths_follow_macos_conventions() {
        let paths = RuntimePaths::packaged_for_home(PathBuf::from("/Users/tester"));
        assert_eq!(
            paths.cache,
            PathBuf::from("/Users/tester/Library/Caches/com.blueblazin.voxelearth")
        );
        assert_eq!(
            paths.photos,
            PathBuf::from("/Users/tester/Pictures/Low Poly Earth")
        );
        assert!(paths.packaged);
    }

    #[test]
    fn window_title_is_product_name_only() {
        assert_eq!(WINDOW_TITLE, PRODUCT_NAME);
    }
}
