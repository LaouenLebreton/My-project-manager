//! Opt-in check of the .mpp path: every `.xml` in `$SUPER_PM_MPP_SAMPLES` (Project XML written
//! by tools/mpp from real .mpp files) must import, and be schedulable. Run with
//! `SUPER_PM_MPP_SAMPLES=/path cargo test -p pm-interop --test mpp_samples -- --ignored --nocapture`.

#[test]
#[ignore = "needs converted .mpp samples; see the module comment"]
fn converted_mpp_files_import() {
    let dir = std::env::var("SUPER_PM_MPP_SAMPLES").expect("set SUPER_PM_MPP_SAMPLES to a folder of converted samples");
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "xml")).collect();
    paths.sort();
    assert!(!paths.is_empty(), "no .xml files in {dir}");
    let mut failures = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut n = 0u128;
        let mut new_id = || {
            n += 1;
            uuid::Uuid::from_u128(n)
        };
        match pm_interop::import_mspdi(&text, &mut new_id) {
            Ok((project, report)) => {
                let scheduled = pm_core::schedule(&project).map(|_| "scheduled").unwrap_or("NOT schedulable");
                let ignored: Vec<String> = report.ignored.iter().map(|l| format!("{} ({})", l.what, l.count)).collect();
                println!("{name}: {} tasks, {} links, {scheduled}; ignored: {}", project.tasks.len(), project.dependencies.len(), ignored.join(", "));
                if scheduled != "scheduled" {
                    failures.push(name);
                }
            }
            Err(e) => {
                println!("{name}: FAILED: {e}");
                failures.push(name);
            }
        }
    }
    assert!(failures.is_empty(), "these did not import: {failures:?}");
}
