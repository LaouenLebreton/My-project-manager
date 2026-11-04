//! Microsoft Project `.mpp` files (iteration 4): the bundled converter (tools/mpp: MPXJ on a
//! small Java runtime) turns one into Project XML, which then imports like any other, with
//! the same checks and import report.

use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::{AppHandle, Manager};

use crate::ipc::IpcError;

/// The bundled runtime's `java`, inside the converter's folder.
const JAVA: &str = if cfg!(windows) { "runtime/bin/java.exe" } else { "runtime/bin/java" };

fn err(message: impl Into<String>) -> IpcError {
    IpcError { message: message.into() }
}

/// Where the converter is: inside the app (Resources/mpp) when bundled; in development, the
/// one `tools/mpp/build.sh` makes in the source tree.
fn converter(app: &AppHandle) -> Option<PathBuf> {
    let bundled = app.path().resource_dir().ok().map(|d| d.join("mpp"));
    let source_tree = cfg!(debug_assertions).then(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tools/mpp/out"));
    bundled.into_iter().chain(source_tree).find(|d| d.join(JAVA).is_file())
}

pub fn is_mpp(path: &Path) -> bool {
    path.extension().is_some_and(|x| x.eq_ignore_ascii_case("mpp"))
}

/// Converts an `.mpp` file to Project XML text. Errors are sentences for the user.
pub fn to_xml(app: &AppHandle, source: &Path) -> Result<String, IpcError> {
    let Some(dir) = converter(app) else {
        return Err(err(
            "this copy of Super PM cannot read .mpp files. In Microsoft Project, choose File > Save As, save as XML, and import that file instead",
        ));
    };
    convert(&dir, source)
}

/// Runs the converter in `dir` (holding `runtime/` and `lib/`) on one file.
fn convert(dir: &Path, source: &Path) -> Result<String, IpcError> {
    let out = std::env::temp_dir().join(format!("super-pm-mpp-{}.xml", uuid::Uuid::now_v7()));
    let mut command = Command::new(dir.join(JAVA));
    // No console window flashing up on Windows.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    let result = command
        // Settings meant for other Java programs must not change this one or its messages.
        .env_remove("JAVA_TOOL_OPTIONS")
        .env_remove("JDK_JAVA_OPTIONS")
        .env_remove("_JAVA_OPTIONS")
        .arg("-Dlog4j2.statusLoggerLevel=OFF")
        .arg("-Djava.awt.headless=true")
        .arg("-cp")
        .arg(dir.join("lib/*"))
        .arg("MppToXml")
        .arg(source)
        .arg(&out)
        .output();
    let output = result.map_err(|e| err(format!("the .mpp converter could not start: {e}")))?;
    let text = if output.status.success() { std::fs::read_to_string(&out).map_err(|e| err(format!("the converted file could not be read: {e}"))) } else {
        // The converter's last line says why, in plain words.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let why = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("the converter stopped without saying why").trim().to_owned();
        Err(err(why))
    };
    let _ = std::fs::remove_file(&out);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The converter built by tools/mpp/build.sh, if it has been built on this machine.
    fn built() -> Option<PathBuf> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tools/mpp/out");
        dir.join(JAVA).is_file().then_some(dir)
    }

    #[test]
    fn a_file_that_is_not_a_project_is_refused_in_plain_words() {
        let Some(dir) = built() else { return eprintln!("skipped: tools/mpp/build.sh has not been run") };
        let bad = std::env::temp_dir().join(format!("super-pm-not-a-project-{}.mpp", uuid::Uuid::now_v7()));
        std::fs::write(&bad, "not a project").unwrap();
        let e = convert(&dir, &bad).unwrap_err();
        let _ = std::fs::remove_file(&bad);
        assert!(e.message.starts_with("this is not a Microsoft Project file Super PM can read"), "{}", e.message);
    }

    #[test]
    #[ignore = "needs sample .mpp files: set SUPER_PM_MPP_SAMPLES"]
    fn sample_files_convert_and_import() {
        let dir = built().expect("run tools/mpp/build.sh first");
        let samples = std::env::var("SUPER_PM_MPP_SAMPLES").expect("set SUPER_PM_MPP_SAMPLES to a folder of .mpp files");
        let mut count = 0;
        for entry in std::fs::read_dir(samples).unwrap() {
            let path = entry.unwrap().path();
            if !is_mpp(&path) {
                continue;
            }
            let xml = convert(&dir, &path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e.message));
            let (project, _) = pm_interop::import_mspdi(&xml, &mut crate::document::new_id).unwrap();
            assert!(!project.tasks.is_empty(), "{}", path.display());
            count += 1;
        }
        assert!(count > 0);
    }
}
