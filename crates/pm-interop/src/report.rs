//! What an import did with each part of the file.

use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// What arrived as it was in Microsoft Project.
    pub imported: Vec<ReportLine>,
    /// What arrived changed, with how.
    pub approximated: Vec<ReportLine>,
    /// What was left out, with why.
    pub ignored: Vec<ReportLine>,
    /// Calculated fields in the file that Super PM calculates itself.
    pub recalculated: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportLine {
    pub what: String,
    pub count: usize,
    /// Up to five task names (or values) as examples.
    pub examples: Vec<String>,
}

const EXAMPLES: usize = 5;

/// Collects report lines, merging repeats of the same message.
#[derive(Default)]
pub struct ReportBuilder {
    imported: BTreeMap<String, (usize, Vec<String>)>,
    approximated: BTreeMap<String, (usize, Vec<String>)>,
    ignored: BTreeMap<String, (usize, Vec<String>)>,
    recalculated: std::collections::BTreeSet<String>,
}

fn add(map: &mut BTreeMap<String, (usize, Vec<String>)>, what: &str, example: Option<&str>) {
    let entry = map.entry(what.to_owned()).or_default();
    entry.0 += 1;
    if let Some(e) = example
        && entry.1.len() < EXAMPLES
        && !entry.1.iter().any(|x| x == e)
    {
        entry.1.push(e.to_owned());
    }
}

impl ReportBuilder {
    pub fn imported(&mut self, what: &str, count: usize) {
        if count > 0 {
            self.imported.insert(what.to_owned(), (count, vec![]));
        }
    }

    pub fn approximated(&mut self, what: &str, example: Option<&str>) {
        add(&mut self.approximated, what, example);
    }

    pub fn ignored(&mut self, what: &str, example: Option<&str>) {
        add(&mut self.ignored, what, example);
    }

    pub fn recalculated(&mut self, field: &str) {
        self.recalculated.insert(field.to_owned());
    }

    pub fn finish(self) -> ImportReport {
        let lines = |m: BTreeMap<String, (usize, Vec<String>)>| {
            m.into_iter().map(|(what, (count, examples))| ReportLine { what, count, examples }).collect()
        };
        ImportReport {
            imported: lines(self.imported),
            approximated: lines(self.approximated),
            ignored: lines(self.ignored),
            recalculated: self.recalculated.into_iter().collect(),
        }
    }
}
